//! Windows notification-area integration for the settings window.
//!
//! `tray-icon` owns a hidden Win32 window, so the icon must be created, polled,
//! and destroyed on the same thread as winit's event loop. Keeping the handle in
//! Bevy's non-Send storage makes every system that touches it run on the main
//! thread.

use bevy::{app::AppExit, prelude::*};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows::{
    Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId, IsIconic},
    core::PCWSTR,
};

use super::{PendingSettingsFocus, SETTINGS_WINDOW_TITLE, SettingsWindow, toggle_settings_window};

const TRAY_ICON_ID: &str = "sunk-system-tray";
const SHOW_MENU_ID: &str = "sunk-show-settings";
const QUIT_MENU_ID: &str = "sunk-quit";

pub(super) struct SystemTrayPlugin;

impl Plugin for SystemTrayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, initialize_system_tray)
            .add_systems(
                Update,
                (
                    hide_minimized_settings,
                    poll_system_tray.after(toggle_settings_window),
                ),
            );
    }
}

/// Non-Send because both `TrayIcon` and its `muda` menu contain `Rc` handles.
struct SystemTrayState {
    icon: TrayIcon,
    show_id: MenuId,
    quit_id: MenuId,
}

impl SystemTrayState {
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

impl Drop for SystemTrayState {
    fn drop(&mut self) {
        // Hide immediately during shutdown; dropping `TrayIcon` directly after
        // this issues NIM_DELETE and destroys its hidden Win32 message window.
        let _ = self.icon.set_visible(false);
    }
}

/// Startup is an exclusive system, so creation occurs on the winit/main thread
/// after the native event loop has started. The value stays in non-Send storage
/// for the same-thread lifetime required by `tray-icon` on Windows.
fn initialize_system_tray(world: &mut World) {
    match SystemTrayState::create() {
        Ok(tray) => world.insert_non_send(tray),
        Err(error) => error!("Windows system tray unavailable: {error}"),
    }
}

/// Winit does not expose a portable minimized-state getter. On Windows,
/// `IsIconic` is the authoritative check; hiding the Bevy window removes its
/// taskbar button while the persistent notification-area icon remains.
fn hide_minimized_settings(
    mut pending: ResMut<PendingSettingsFocus>,
    mut settings_window: Query<&mut Window, With<SettingsWindow>>,
) {
    let Ok(mut window) = settings_window.single_mut() else {
        return;
    };
    if !window.visible || !settings_window_is_minimized() {
        return;
    }

    // Clear the native iconic state before hiding. This guarantees a later
    // tray restore returns to a normal window instead of a hidden minimized one.
    window.set_minimized(false);
    window.visible = false;
    window.focused = false;
    pending.0 = false;
}

fn poll_system_tray(
    tray: Option<NonSend<SystemTrayState>>,
    mut pending: ResMut<PendingSettingsFocus>,
    mut settings_window: Query<&mut Window, With<SettingsWindow>>,
    mut app_exit: MessageWriter<AppExit>,
) {
    let Some(tray) = tray else {
        return;
    };

    let mut show_requested = false;
    let mut quit_requested = false;

    for event in TrayIconEvent::receiver().try_iter() {
        if event.id() != tray.icon.id() {
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
        if event.id == tray.show_id {
            show_requested = true;
        } else if event.id == tray.quit_id {
            quit_requested = true;
        }
    }

    if quit_requested {
        app_exit.write(AppExit::Success);
        return;
    }

    if show_requested {
        let Ok(mut window) = settings_window.single_mut() else {
            return;
        };
        window.set_minimized(false);
        window.visible = true;
        // `poll_system_tray` is ordered after the keyboard toggle, whose chain
        // starts with `apply_pending_focus`. Focus is therefore requested on the
        // following frame, after winit has made the native HWND visible.
        pending.0 = true;
    }
}

fn settings_window_is_minimized() -> bool {
    let title: Vec<u16> = SETTINGS_WINDOW_TITLE
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: `title` is NUL-terminated and valid for this call. A null class
    // name asks Windows to match only the window title.
    let Ok(hwnd) = (unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }) else {
        return false;
    };

    let mut process_id = 0_u32;
    // SAFETY: `process_id` is writable and `hwnd` came from FindWindowW.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if thread_id == 0 || process_id != std::process::id() {
        return false;
    }

    // SAFETY: The HWND was found above and belongs to this process.
    unsafe { IsIconic(hwnd).as_bool() }
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
