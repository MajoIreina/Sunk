//! Windows notification-area integration for the settings window.
//!
//! `tray-icon` owns a hidden Win32 window, so the icon must be created, polled,
//! and destroyed on the same thread as winit's event loop. Keeping the handle in
//! Bevy's non-Send storage makes every system that touches it run on the main
//! thread.

use std::sync::atomic::{AtomicBool, Ordering};

use bevy::{app::AppExit, prelude::*};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                FindWindowW, GetWindowThreadProcessId, IsIconic, IsWindow, SC_MINIMIZE,
                SIZE_MINIMIZED, SW_HIDE, SW_RESTORE, ShowWindow, WM_SIZE, WM_SYSCOMMAND,
            },
        },
    },
    core::PCWSTR,
};

use super::{PendingSettingsFocus, SETTINGS_WINDOW_TITLE, SettingsWindow, hide_settings_on_close};

const TRAY_ICON_ID: &str = "sunk-system-tray";
const SHOW_MENU_ID: &str = "sunk-show-settings";
const QUIT_MENU_ID: &str = "sunk-quit";
const SETTINGS_SUBCLASS_ID: usize = 0x5355_4E4B;

pub(super) struct SystemTrayPlugin;

impl Plugin for SystemTrayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, initialize_system_tray)
            .add_systems(Update, poll_system_tray.after(hide_settings_on_close));
    }
}

struct SystemTrayState {
    tray_ui: Option<TrayUi>,
    settings_hwnd: Option<HWND>,
    native_hide_requested: &'static AtomicBool,
}

/// Non-Send because both `TrayIcon` and its `muda` menu contain `Rc` handles.
struct TrayUi {
    icon: TrayIcon,
    show_id: MenuId,
    quit_id: MenuId,
}

impl TrayUi {
    fn create() -> Result<Self, String> {
        let show = MenuItem::with_id(SHOW_MENU_ID, "显示设置", true, None);
        let separator = PredefinedMenuItem::separator();
        let quit = MenuItem::with_id(QUIT_MENU_ID, "退出 Sunk", true, None);
        let menu = Menu::with_items(&[&show, &separator, &quit])
            .map_err(|error| format!("could not create tray menu: {error}"))?;

        let icon = TrayIconBuilder::new()
            .with_id(TRAY_ICON_ID)
            .with_tooltip("Sunk 黑洞 - 单击打开设置")
            .with_icon(make_black_hole_icon()?)
            .with_menu(Box::new(menu))
            // A left click restores immediately. The context menu remains on
            // right click and exposes the same Show action plus Quit.
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|error| format!("could not create tray icon: {error}"))?;

        Ok(Self {
            icon,
            show_id: show.id().clone(),
            quit_id: quit.id().clone(),
        })
    }
}

impl SystemTrayState {
    fn create() -> Self {
        let tray_ui = match TrayUi::create() {
            Ok(tray) => Some(tray),
            Err(error) => {
                error!("Windows system tray unavailable: {error}");
                None
            }
        };

        Self {
            tray_ui,
            settings_hwnd: None,
            // The callback may outlive normal ECS teardown if Windows has
            // already started destroying HWNDs. One process-lifetime atomic is
            // intentionally leaked so dwRefData can never become dangling.
            native_hide_requested: Box::leak(Box::new(AtomicBool::new(false))),
        }
    }

    fn install_settings_subclass(&mut self) {
        if let Some(hwnd) = self.settings_hwnd {
            // SAFETY: IsWindow only queries the cached opaque handle.
            if unsafe { IsWindow(Some(hwnd)).as_bool() } {
                return;
            }
            self.settings_hwnd = None;
        }
        let Some(hwnd) = find_settings_hwnd() else {
            return;
        };
        let state = self.native_hide_requested as *const AtomicBool as usize;
        // SAFETY: The callback pointer and subclass id stay constant, and state
        // has process lifetime. This system runs on the window event-loop thread.
        if unsafe {
            SetWindowSubclass(
                hwnd,
                Some(settings_window_subclass),
                SETTINGS_SUBCLASS_ID,
                state,
            )
        }
        .as_bool()
        {
            self.settings_hwnd = Some(hwnd);
        } else {
            warn!("could not intercept settings-window minimize messages");
        }
    }

    fn restore_settings_window(&mut self) {
        self.install_settings_subclass();
        let Some(hwnd) = self.settings_hwnd else {
            return;
        };
        // Restore explicitly only for fallback paths that became iconic before
        // the subclass was installed. Normally hidden windows are shown through
        // Bevy below, avoiding a synchronous native show before a rendered frame.
        // SAFETY: The cached HWND was validated by install_settings_subclass.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }
    }
}

impl Drop for SystemTrayState {
    fn drop(&mut self) {
        if let Some(hwnd) = self.settings_hwnd.take() {
            // SAFETY: This uses the same HWND, callback and id passed to
            // SetWindowSubclass, and runs on the main thread that installed it.
            unsafe {
                let _ = RemoveWindowSubclass(
                    hwnd,
                    Some(settings_window_subclass),
                    SETTINGS_SUBCLASS_ID,
                );
            }
        }
        if let Some(tray) = self.tray_ui.as_ref() {
            // Hide immediately during shutdown; dropping `TrayIcon` directly
            // after this issues NIM_DELETE and destroys its hidden message window.
            let _ = tray.icon.set_visible(false);
        }
    }
}

/// Consume minimize before it reaches winit/wgpu. DX12 otherwise starts a
/// zero-sized surface reconfiguration while the next Bevy frame tries to
/// restore and hide the same HWND, which can fail ResizeBuffers fatally.
unsafe extern "system" fn settings_window_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    state: usize,
) -> LRESULT {
    let minimize_command = message == WM_SYSCOMMAND && (wparam.0 as u32 & 0xFFF0) == SC_MINIMIZE;
    let minimized_size = message == WM_SIZE && wparam.0 as u32 == SIZE_MINIMIZED;
    if minimize_command || minimized_size {
        // SAFETY: `state` points to the process-lifetime AtomicBool installed above.
        let hidden = unsafe { &*(state as *const AtomicBool) };
        hidden.store(true, Ordering::Release);
        // SAFETY: `hwnd` is the window currently dispatching this message.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        return LRESULT(0);
    }

    // SAFETY: Every message not explicitly consumed must continue through the
    // comctl32 subclass chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// Startup is an exclusive system, so creation occurs on the winit/main thread
/// after the native event loop has started. The value stays in non-Send storage
/// for the same-thread lifetime required by `tray-icon` on Windows.
fn initialize_system_tray(world: &mut World) {
    // Always install the minimize guard. Tray creation is best effort and must
    // not decide whether minimizing the settings surface is process-safe.
    world.insert_non_send(SystemTrayState::create());
}

fn poll_system_tray(
    tray: Option<NonSendMut<SystemTrayState>>,
    mut pending: ResMut<PendingSettingsFocus>,
    mut settings_window: Query<&mut Window, With<SettingsWindow>>,
    mut app_exit: MessageWriter<AppExit>,
) {
    let Some(mut tray) = tray else {
        return;
    };

    tray.install_settings_subclass();

    let native_hidden = tray.native_hide_requested.swap(false, Ordering::AcqRel);
    if native_hidden {
        let Ok(mut window) = settings_window.single_mut() else {
            return;
        };
        // The HWND is already hidden. Updating Bevy's mirror once keeps Egui
        // and rendering asleep without issuing the minimize/restore sequence.
        window.visible = false;
        window.focused = false;
        pending.0 = false;
    }

    let mut show_requested = false;
    let mut quit_requested = false;

    if let Some(tray_ui) = tray.tray_ui.as_ref() {
        for event in TrayIconEvent::receiver().try_iter() {
            if event.id() != tray_ui.icon.id() {
                continue;
            }
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_requested = true;
            }
        }

        for event in MenuEvent::receiver().try_iter() {
            if event.id == tray_ui.show_id {
                show_requested = true;
            } else if event.id == tray_ui.quit_id {
                quit_requested = true;
            }
        }
    }

    if quit_requested {
        app_exit.write(AppExit::Success);
        return;
    }

    if show_requested {
        tray.restore_settings_window();
        let Ok(mut window) = settings_window.single_mut() else {
            return;
        };
        window.visible = true;
        // `poll_system_tray` is ordered after the keyboard toggle, whose chain
        // starts with `apply_pending_focus`. Focus is therefore requested on the
        // following frame, after winit has made the native HWND visible.
        pending.0 = true;
    }
}

fn find_settings_hwnd() -> Option<HWND> {
    let title: Vec<u16> = SETTINGS_WINDOW_TITLE
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: `title` is NUL-terminated and valid for this call. A null class
    // name asks Windows to match only the window title.
    let hwnd = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }.ok()?;

    let mut process_id = 0_u32;
    // SAFETY: `process_id` is writable and `hwnd` came from FindWindowW.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if thread_id == 0 || process_id != std::process::id() {
        return None;
    }
    Some(hwnd)
}

fn make_black_hole_icon() -> Result<Icon, String> {
    const SIZE: u32 = 64;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let center = (SIZE as f32 - 1.0) * 0.5;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let radius = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);

            let core = (1.0 - ((radius - 13.0) / 2.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
            let ring_distance = (radius - 21.0) / 3.4;
            let ring = (-ring_distance * ring_distance).exp();
            let flow = 0.72 + 0.28 * (angle * 3.0 + radius * 0.28).sin();
            let glow = (ring * flow).clamp(0.0, 1.0);
            let photon_rim = (-((radius - 14.8) / 1.15).powi(2)).exp();

            let red = (24.0 + 231.0 * glow + 90.0 * photon_rim).clamp(0.0, 255.0);
            let green = (10.0 + 119.0 * glow + 145.0 * photon_rim).clamp(0.0, 255.0);
            let blue = (18.0 + 34.0 * glow + 150.0 * photon_rim).clamp(0.0, 255.0);
            let alpha = ((core * 0.96 + glow * 0.92 + photon_rim).clamp(0.0, 1.0) * 255.0) as u8;

            rgba.extend_from_slice(&[red as u8, green as u8, blue as u8, alpha]);
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE).map_err(|error| format!("invalid tray icon pixels: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_icon_has_the_expected_dimensions() {
        make_black_hole_icon().expect("procedural tray icon should be valid");
    }
}
