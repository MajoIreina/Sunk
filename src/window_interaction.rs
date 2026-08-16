//! Keeps the transparent renderer fitted to its content and click-through elsewhere.

use bevy::{
    input::{ButtonState, mouse::MouseButtonInput},
    prelude::*,
    window::{CursorOptions, PrimaryWindow},
};

use crate::{
    black_hole::BlackHoleControls, file_interaction::DropInteractionState,
    settings::BlackHoleSettings,
};

const PRIMARY_WINDOW_TITLE: &str = "Sunk Black Hole";
// `WindowResolution::new` defines the initial surface in physical pixels. Keep
// this baseline physical as well, or per-monitor DPI would be applied twice.
const BASE_CLIENT_WIDTH: f32 = 900.0;
const BASE_CLIENT_HEIGHT: f32 = 700.0;
const BASE_VERTICAL_FOV_RADIANS: f32 = 42.0_f32.to_radians();
const CAMERA_DISTANCE_RS: f32 = 30.0;
const DISK_OUTER_RADIUS_RS: f32 = 11.5;
const CRITICAL_IMPACT_PARAMETER_RS: f32 = 2.598_076;
const BACKGROUND_INFLUENCE_SHADOW_RADII: f32 = 3.45;
const DISK_LENSING_PADDING: f32 = 1.10;
const CONTENT_SAFE_RADIUS: f32 = 0.88;
const MIN_COMPOSITION_SCALE: f32 = 1.25;
const RESIZE_INTERVAL_SECONDS: f64 = 0.08;
const LEFT_POINTER_BUTTON: u8 = 1 << 0;
const RIGHT_POINTER_BUTTON: u8 = 1 << 1;

pub(crate) struct WindowInteractionPlugin;

impl Plugin for WindowInteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OverlayWindowRuntime>();

        #[cfg(target_os = "windows")]
        {
            app.add_systems(Startup, windows_backend::initialize_external_drag_hook)
                .add_systems(
                    PostUpdate,
                    (
                        windows_backend::fit_primary_window_to_black_hole,
                        windows_backend::update_primary_window_hit_test,
                    )
                        .chain(),
                );
        }
    }
}

/// Expands the camera frustum and the native window by the same factor. The
/// rendered object therefore keeps its requested physical size while retaining
/// transparent padding at every supported lens-radius setting.
pub(crate) fn render_tan_half_fov(lens_radius_scale: f32) -> f32 {
    (BASE_VERTICAL_FOV_RADIANS * 0.5).tan() * composition_scale(lens_radius_scale)
}

fn composition_scale(lens_radius_scale: f32) -> f32 {
    let base_tan_half_fov = (BASE_VERTICAL_FOV_RADIANS * 0.5).tan();
    let disk_extent =
        DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING / (CAMERA_DISTANCE_RS * base_tan_half_fov);
    let lens_extent = CRITICAL_IMPACT_PARAMETER_RS
        * BACKGROUND_INFLUENCE_SHADOW_RADII
        * lens_radius_scale.clamp(0.4, 2.0)
        / (CAMERA_DISTANCE_RS * base_tan_half_fov);

    MIN_COMPOSITION_SCALE.max(disk_extent.max(lens_extent) / CONTENT_SAFE_RADIUS)
}

fn target_client_size(
    apparent_size: f32,
    lens_radius_scale: f32,
    available_client_size: UVec2,
) -> (UVec2, f32) {
    let requested_apparent_size = apparent_size.clamp(0.25, 3.0);
    let composition_scale = composition_scale(lens_radius_scale);
    let base_physical = Vec2::new(BASE_CLIENT_WIDTH, BASE_CLIENT_HEIGHT);
    let available = available_client_size.as_vec2().max(Vec2::splat(64.0));
    let maximum_scale = (available.x / base_physical.x)
        .min(available.y / base_physical.y)
        .max(64.0 / base_physical.min_element());
    let effective_apparent_size = requested_apparent_size.min(maximum_scale / composition_scale);
    let fitted_scale = effective_apparent_size * composition_scale;

    (
        (base_physical * fitted_scale)
            .round()
            .max(Vec2::splat(64.0))
            .as_uvec2(),
        effective_apparent_size,
    )
}

fn cursor_hits_black_hole(
    cursor: Vec2,
    client_size: Vec2,
    pitch: f32,
    lens_radius_scale: f32,
) -> bool {
    if client_size.x <= 1.0 || client_size.y <= 1.0 {
        return false;
    }

    let aspect = client_size.x / client_size.y;
    let screen = Vec2::new(
        (cursor.x / client_size.x * 2.0 - 1.0) * aspect,
        1.0 - cursor.y / client_size.y * 2.0,
    );
    let tan_half_fov = render_tan_half_fov(lens_radius_scale);
    // Pure desktop refraction remains click-through. Only the black shadow,
    // photon ring, and emitting material participate in pointer hit testing.
    let shadow_radius = CRITICAL_IMPACT_PARAMETER_RS / (CAMERA_DISTANCE_RS * tan_half_fov) * 1.18;
    if screen.length_squared() <= shadow_radius * shadow_radius {
        return true;
    }

    // The disk's major axis stays horizontal because yaw rotates around its
    // normal. Its projected minor axis grows with pitch; the shadow circle forms
    // the lower bound so the photon ring remains clickable.
    let disk_major = (DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING
        / (CAMERA_DISTANCE_RS * tan_half_fov))
        .min(0.98);
    let projected_disk_minor = disk_major * (pitch.abs().sin() + 0.10).min(1.0);
    let disk_minor = projected_disk_minor.max(shadow_radius);
    let ellipse_distance = (screen.x / disk_major).powi(2) + (screen.y / disk_minor).powi(2);
    ellipse_distance <= 1.0
}

fn cursor_hits_lens_influence(cursor: Vec2, client_size: Vec2, lens_radius_scale: f32) -> bool {
    if !cursor.is_finite()
        || !client_size.is_finite()
        || !lens_radius_scale.is_finite()
        || client_size.x <= 1.0
        || client_size.y <= 1.0
    {
        return false;
    }

    let aspect = client_size.x / client_size.y;
    let screen = Vec2::new(
        (cursor.x / client_size.x * 2.0 - 1.0) * aspect,
        1.0 - cursor.y / client_size.y * 2.0,
    );
    let shadow_radius = CRITICAL_IMPACT_PARAMETER_RS
        / (CAMERA_DISTANCE_RS * render_tan_half_fov(lens_radius_scale));
    let influence_radius =
        (shadow_radius * BACKGROUND_INFLUENCE_SHADOW_RADII * lens_radius_scale.clamp(0.4, 2.0))
            .clamp(0.12, 3.0);
    let radius_squared = influence_radius * influence_radius;
    let boundary_tolerance = radius_squared * (8.0 * f32::EPSILON);
    screen.length_squared() <= radius_squared + boundary_tolerance
}

fn client_size_matches(client_size: IVec2, target_size: UVec2) -> bool {
    (client_size.x - target_size.x as i32).abs() <= 1
        && (client_size.y - target_size.y as i32).abs() <= 1
}

fn pointer_button_flag(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => LEFT_POINTER_BUTTON,
        MouseButton::Right => RIGHT_POINTER_BUTTON,
        _ => 0,
    }
}

fn should_defer_window_resize(pointer_pressed: bool, file_drag_active: bool) -> bool {
    pointer_pressed || file_drag_active
}

fn should_hit_test_overlay(pointer_locked: bool, cursor_hit: bool, file_drag_active: bool) -> bool {
    pointer_locked || cursor_hit || file_drag_active
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalDragTransition {
    Start,
    End,
    PointerSample { left_button_down: bool },
}

#[cfg(any(target_os = "windows", test))]
fn transition_external_drag(active: bool, transition: ExternalDragTransition) -> bool {
    match transition {
        ExternalDragTransition::Start => true,
        ExternalDragTransition::End => false,
        ExternalDragTransition::PointerSample { left_button_down } => active && left_button_down,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PrimaryCursorSample {
    pub(crate) position: Vec2,
    pub(crate) client_size: Vec2,
}

fn logical_cursor_sample(
    physical_position: Vec2,
    physical_client_size: Vec2,
    scale_factor: f32,
) -> Option<PrimaryCursorSample> {
    let scale = scale_factor;
    if !physical_position.is_finite()
        || !physical_client_size.is_finite()
        || !scale.is_finite()
        || scale <= 0.0
        || physical_client_size.x <= 1.0
        || physical_client_size.y <= 1.0
        || physical_position.x < 0.0
        || physical_position.y < 0.0
        || physical_position.x >= physical_client_size.x
        || physical_position.y >= physical_client_size.y
    {
        return None;
    }

    Some(PrimaryCursorSample {
        position: physical_position / scale,
        client_size: physical_client_size / scale,
    })
}

#[derive(Resource, Debug)]
pub(crate) struct OverlayWindowRuntime {
    native_handle: usize,
    last_resize_at: f64,
    last_reported_limit: Option<f32>,
    resize_pending: bool,
    owned_pointer_buttons: u8,
    pointer_locked: bool,
}

impl OverlayWindowRuntime {
    fn update_pointer_button(
        &mut self,
        button: MouseButton,
        state: ButtonState,
        primary_content_press: bool,
    ) {
        let flag = pointer_button_flag(button);
        if flag == 0 {
            return;
        }

        if state.is_pressed() {
            if primary_content_press {
                self.owned_pointer_buttons |= flag;
            }
        } else {
            self.owned_pointer_buttons &= !flag;
        }
        self.pointer_locked = self.owned_pointer_buttons != 0;
    }

    fn release_unpressed_pointer_buttons(&mut self, buttons: &ButtonInput<MouseButton>) {
        if !buttons.pressed(MouseButton::Left) {
            self.owned_pointer_buttons &= !LEFT_POINTER_BUTTON;
        }
        if !buttons.pressed(MouseButton::Right) {
            self.owned_pointer_buttons &= !RIGHT_POINTER_BUTTON;
        }
        self.pointer_locked = self.owned_pointer_buttons != 0;
    }

    fn clear_pointer_lock(&mut self) {
        self.owned_pointer_buttons = 0;
        self.pointer_locked = false;
    }
}

impl Default for OverlayWindowRuntime {
    fn default() -> Self {
        Self {
            native_handle: 0,
            last_resize_at: f64::NEG_INFINITY,
            last_reported_limit: None,
            resize_pending: false,
            owned_pointer_buttons: 0,
            pointer_locked: false,
        }
    }
}

/// Samples the current native cursor position instead of Bevy's `CursorMoved`
/// cache. Windows OLE drag-over and drop callbacks do not update that cache.
pub(crate) fn primary_cursor_sample(
    window: &Window,
    runtime: &mut OverlayWindowRuntime,
) -> Option<PrimaryCursorSample> {
    #[cfg(target_os = "windows")]
    {
        let hwnd = windows_backend::owned_primary_hwnd(runtime)?;
        let (physical_position, physical_client_size) = windows_backend::cursor_in_client(hwnd)?;
        logical_cursor_sample(
            physical_position,
            physical_client_size,
            window.scale_factor(),
        )
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = runtime;
        let position = window.cursor_position()?;
        let client_size = Vec2::new(window.width(), window.height());
        (position.x >= 0.0
            && position.y >= 0.0
            && position.x < client_size.x
            && position.y < client_size.y)
            .then_some(PrimaryCursorSample {
                position,
                client_size,
            })
    }
}

#[cfg(target_os = "windows")]
mod windows_backend {
    use std::{
        ffi::c_void,
        mem::size_of,
        sync::atomic::{AtomicBool, Ordering},
    };

    use bevy::prelude::*;
    use windows::{
        Win32::{
            Foundation::{HWND, POINT, RECT},
            Graphics::Gdi::{
                GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
                ScreenToClient,
            },
            UI::{
                Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
                Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
                WindowsAndMessaging::{
                    EVENT_SYSTEM_DRAGDROPEND, EVENT_SYSTEM_DRAGDROPSTART, FindWindowW,
                    GetClientRect, GetCursorPos, GetWindowRect, GetWindowThreadProcessId,
                    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
                },
            },
        },
        core::PCWSTR,
    };

    use super::*;

    static EXTERNAL_DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);

    pub(super) struct ExternalDragHook {
        handle: Option<HWINEVENTHOOK>,
    }

    impl ExternalDragHook {
        fn install() -> Self {
            EXTERNAL_DRAG_ACTIVE.store(false, Ordering::Release);
            // SAFETY: The callback is a process-lifetime function and does not
            // capture Rust references. Out-of-context delivery runs through the
            // installing UI thread's message loop.
            let handle = unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_DRAGDROPSTART,
                    EVENT_SYSTEM_DRAGDROPEND,
                    None,
                    Some(external_drag_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            if handle.is_invalid() {
                warn!(
                    "external drag detection is unavailable: {}",
                    windows::core::Error::from_thread()
                );
                Self { handle: None }
            } else {
                Self {
                    handle: Some(handle),
                }
            }
        }

        fn poll_active(&self) -> bool {
            if self.handle.is_none() {
                return false;
            }
            let active = EXTERNAL_DRAG_ACTIVE.load(Ordering::Acquire);
            if !active {
                return false;
            }
            let next = transition_external_drag(
                active,
                ExternalDragTransition::PointerSample {
                    left_button_down: left_button_is_down(),
                },
            );
            if active != next {
                EXTERNAL_DRAG_ACTIVE.store(next, Ordering::Release);
            }
            next
        }
    }

    impl Drop for ExternalDragHook {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take()
                // SAFETY: This is the exact handle returned by SetWinEventHook,
                // retained by this main-thread non-Send resource until teardown.
                && !unsafe { UnhookWinEvent(handle) }.as_bool()
            {
                warn!(
                    "could not remove external drag event hook: {}",
                    windows::core::Error::from_thread()
                );
            }
            EXTERNAL_DRAG_ACTIVE.store(false, Ordering::Release);
        }
    }

    unsafe extern "system" fn external_drag_event(
        _hook: HWINEVENTHOOK,
        event: u32,
        _hwnd: HWND,
        _object_id: i32,
        _child_id: i32,
        _event_thread: u32,
        _event_time_ms: u32,
    ) {
        let transition = match event {
            EVENT_SYSTEM_DRAGDROPSTART => ExternalDragTransition::Start,
            EVENT_SYSTEM_DRAGDROPEND => ExternalDragTransition::End,
            _ => return,
        };
        let active = EXTERNAL_DRAG_ACTIVE.load(Ordering::Acquire);
        EXTERNAL_DRAG_ACTIVE.store(
            transition_external_drag(active, transition),
            Ordering::Release,
        );
    }

    pub(super) fn initialize_external_drag_hook(world: &mut World) {
        world.insert_non_send(ExternalDragHook::install());
    }

    pub(super) fn fit_primary_window_to_black_hole(
        time: Res<Time>,
        settings: Res<BlackHoleSettings>,
        mouse_buttons: Res<ButtonInput<MouseButton>>,
        file_drag: Res<DropInteractionState>,
        external_drag: Option<NonSend<ExternalDragHook>>,
        mut window: Single<&mut Window, With<PrimaryWindow>>,
        mut runtime: ResMut<OverlayWindowRuntime>,
    ) {
        let Some(hwnd) = owned_primary_hwnd(&mut runtime) else {
            return;
        };
        let Ok(geometry) = native_geometry(hwnd) else {
            runtime.native_handle = 0;
            return;
        };

        let chrome = IVec2::new(
            geometry.outer_width - geometry.client_width,
            geometry.outer_height - geometry.client_height,
        )
        .max(IVec2::ZERO);
        let work_size = IVec2::new(
            geometry.work.right - geometry.work.left,
            geometry.work.bottom - geometry.work.top,
        );
        let available_client = (work_size - chrome).max(IVec2::splat(64)).as_uvec2();
        let (target_client, effective_apparent_size) = target_client_size(
            settings.apparent_size,
            settings.lens_radius,
            available_client,
        );
        if (settings.apparent_size - effective_apparent_size).abs() > 1.0e-4 {
            if runtime
                .last_reported_limit
                .is_none_or(|previous| (previous - effective_apparent_size).abs() > 0.01)
            {
                info!(
                    "black-hole size limited to {:.2} by the current monitor work area",
                    effective_apparent_size
                );
                runtime.last_reported_limit = Some(effective_apparent_size);
            }
        } else {
            runtime.last_reported_limit = None;
        }

        if client_size_matches(
            IVec2::new(geometry.client_width, geometry.client_height),
            target_client,
        ) {
            runtime.resize_pending = false;
            return;
        }

        // A previously queued native resize is still allowed to settle and clear
        // `resize_pending`, but no new geometry change starts during pointer or
        // OLE file drags.
        if should_defer_window_resize(
            mouse_buttons.any_pressed([MouseButton::Left, MouseButton::Right]),
            file_drag.drag_active
                || external_drag
                    .as_ref()
                    .is_some_and(|hook| hook.poll_active()),
        ) {
            return;
        }

        if time.elapsed_secs_f64() - runtime.last_resize_at < RESIZE_INTERVAL_SECONDS {
            return;
        }

        let target_outer = target_client.as_ivec2() + chrome;
        let center = IVec2::new(
            geometry.outer.left + geometry.outer_width / 2,
            geometry.outer.top + geometry.outer_height / 2,
        );
        let minimum = IVec2::new(geometry.work.left, geometry.work.top);
        let maximum = IVec2::new(
            geometry.work.right - target_outer.x,
            geometry.work.bottom - target_outer.y,
        );
        let position = (center - target_outer / 2).clamp(minimum, maximum.max(minimum));

        // Queue the native update through Bevy/winit's Last-stage synchronization.
        // Calling SetWindowPos here would synchronously re-enter the same Windows
        // event loop and can deadlock the first frame before the settings HWND exists.
        window.position = bevy::window::WindowPosition::At(position);
        window
            .resolution
            .set_physical_resolution(target_client.x, target_client.y);
        runtime.last_resize_at = time.elapsed_secs_f64();
        runtime.resize_pending = true;
    }

    pub(super) fn update_primary_window_hit_test(
        mouse_buttons: Res<ButtonInput<MouseButton>>,
        mut mouse_button_events: MessageReader<MouseButtonInput>,
        controls: Res<BlackHoleControls>,
        settings: Res<BlackHoleSettings>,
        drag_sources: (Res<DropInteractionState>, Option<NonSend<ExternalDragHook>>),
        window: Single<(Entity, &mut CursorOptions), With<PrimaryWindow>>,
        mut runtime: ResMut<OverlayWindowRuntime>,
    ) {
        let (file_drag, external_drag) = drag_sources;
        let (primary_window, mut cursor_options) = window.into_inner();
        let cursor_sample = owned_primary_hwnd(&mut runtime).and_then(cursor_in_client);
        let cursor_hit = !controls.pass_through()
            && !runtime.resize_pending
            && cursor_sample.is_some_and(|(cursor, client_size)| {
                cursor_hits_black_hole(cursor, client_size, controls.pitch(), settings.lens_radius)
            });
        let external_drag_active = external_drag
            .as_ref()
            .is_some_and(|hook| hook.poll_active());
        let acquiring_external_drag = !controls.pass_through()
            && !runtime.resize_pending
            && !runtime.pointer_locked
            && external_drag_active
            && cursor_sample.is_some_and(|(cursor, client_size)| {
                cursor_hits_lens_influence(cursor, client_size, settings.lens_radius)
            });

        let primary_can_own_press = cursor_options.hit_test && cursor_hit;
        for event in mouse_button_events.read() {
            runtime.update_pointer_button(
                event.button,
                event.state,
                event.window == primary_window && primary_can_own_press,
            );
        }
        // Focus loss or a native drag can occasionally consume the release
        // message. Global button state is used only to release ownership, never
        // to establish it, so input from the settings window cannot lock this one.
        runtime.release_unpressed_pointer_buttons(&mouse_buttons);

        if (controls.pass_through() || runtime.resize_pending) && !file_drag.drag_active {
            runtime.clear_pointer_lock();
            if cursor_options.hit_test {
                cursor_options.hit_test = false;
            }
            return;
        }

        let hit = should_hit_test_overlay(
            runtime.pointer_locked,
            cursor_hit,
            file_drag.drag_active || acquiring_external_drag,
        );

        // Bevy/winit applies this once in `Last`. Avoid marking the component
        // changed every frame, which otherwise repeatedly calls Win32 style APIs.
        if cursor_options.hit_test != hit {
            cursor_options.hit_test = hit;
        }
    }

    struct NativeGeometry {
        outer: RECT,
        work: RECT,
        outer_width: i32,
        outer_height: i32,
        client_width: i32,
        client_height: i32,
    }

    pub(super) fn owned_primary_hwnd(runtime: &mut OverlayWindowRuntime) -> Option<HWND> {
        if runtime.native_handle != 0 {
            let hwnd = HWND(runtime.native_handle as *mut c_void);
            if belongs_to_current_process(hwnd) {
                return Some(hwnd);
            }
            runtime.native_handle = 0;
        }

        let title: Vec<u16> = PRIMARY_WINDOW_TITLE.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `title` is NUL-terminated and alive for the duration of the call.
        let hwnd = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }.ok()?;
        if !belongs_to_current_process(hwnd) {
            return None;
        }
        runtime.native_handle = hwnd.0 as usize;
        Some(hwnd)
    }

    fn belongs_to_current_process(hwnd: HWND) -> bool {
        let mut process_id = 0_u32;
        // SAFETY: `process_id` is writable. Invalid/stale handles yield thread id 0.
        let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
        thread_id != 0 && process_id == std::process::id()
    }

    fn left_button_is_down() -> bool {
        // The high bit reports the current physical state even while Explorer's
        // OLE loop owns the pointer and Bevy receives no mouse-button events.
        unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
    }

    fn native_geometry(hwnd: HWND) -> Result<NativeGeometry, windows::core::Error> {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        // SAFETY: Both rectangles are writable and `hwnd` was validated above.
        unsafe {
            GetWindowRect(hwnd, &mut outer)?;
            GetClientRect(hwnd, &mut client)?;
        }

        // SAFETY: Default-to-nearest guarantees a monitor for a valid top-level HWND.
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let mut monitor_info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..default()
        };
        // SAFETY: `monitor_info` advertises its exact size and is writable.
        if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
            return Err(windows::core::Error::from_thread());
        }

        Ok(NativeGeometry {
            outer,
            work: monitor_info.rcWork,
            outer_width: outer.right - outer.left,
            outer_height: outer.bottom - outer.top,
            client_width: client.right - client.left,
            client_height: client.bottom - client.top,
        })
    }

    pub(super) fn cursor_in_client(hwnd: HWND) -> Option<(Vec2, Vec2)> {
        let mut cursor = POINT::default();
        let mut client = RECT::default();
        // SAFETY: Both output structures are writable and `hwnd` is process-owned.
        unsafe {
            GetCursorPos(&mut cursor).ok()?;
            GetClientRect(hwnd, &mut client).ok()?;
        }
        // SAFETY: Converts the physical screen point in place for this live HWND.
        if !unsafe { ScreenToClient(hwnd, &mut cursor) }.as_bool() {
            return None;
        }

        let width = client.right - client.left;
        let height = client.bottom - client.top;
        if cursor.x < 0 || cursor.y < 0 || cursor.x >= width || cursor.y >= height {
            return None;
        }
        Some((
            Vec2::new(cursor.x as f32, cursor.y as f32),
            Vec2::new(width as f32, height as f32),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens_influence_radius(lens_radius_scale: f32) -> f32 {
        let shadow_radius = CRITICAL_IMPACT_PARAMETER_RS
            / (CAMERA_DISTANCE_RS * render_tan_half_fov(lens_radius_scale));
        (shadow_radius * BACKGROUND_INFLUENCE_SHADOW_RADII * lens_radius_scale.clamp(0.4, 2.0))
            .clamp(0.12, 3.0)
    }

    fn cursor_for_screen_point(screen: Vec2, client_size: Vec2) -> Vec2 {
        let aspect = client_size.x / client_size.y;
        Vec2::new(
            (screen.x / aspect + 1.0) * client_size.x * 0.5,
            (1.0 - screen.y) * client_size.y * 0.5,
        )
    }

    #[test]
    fn client_size_tracks_apparent_size_until_monitor_limit() {
        let roomy = UVec2::new(4_000, 3_000);
        let (normal, normal_scale) = target_client_size(1.0, 1.0, roomy);
        let (doubled, doubled_scale) = target_client_size(2.0, 1.0, roomy);
        assert_eq!(doubled, normal * 2);
        assert_eq!(normal_scale, 1.0);
        assert_eq!(doubled_scale, 2.0);

        let (limited, limited_scale) = target_client_size(3.0, 1.0, UVec2::new(1_920, 1_040));
        assert!(limited.x <= 1_920 && limited.y <= 1_040);
        assert!(limited_scale < 3.0);
        assert!((limited.x as f32 / limited.y as f32 - 900.0 / 700.0).abs() < 0.002);

        // The monitor cap is an effective size, not a replacement for the user's
        // request. Re-evaluating that request on a roomy monitor must recover it.
        let (restored, restored_scale) = target_client_size(3.0, 1.0, roomy);
        assert_eq!(restored_scale, 3.0);
        assert!(restored.x > limited.x && restored.y > limited.y);
    }

    #[test]
    fn native_resize_confirmation_requires_the_target_size() {
        let target = UVec2::new(1_125, 875);
        assert!(client_size_matches(IVec2::new(1_125, 875), target));
        assert!(client_size_matches(IVec2::new(1_124, 876), target));
        assert!(!client_size_matches(IVec2::new(1_123, 875), target));
        assert!(!client_size_matches(IVec2::new(1_125, 877), target));
    }

    #[test]
    fn pointer_lock_requires_a_primary_content_press() {
        let mut runtime = OverlayWindowRuntime::default();

        // A press delivered to the settings window must never make the primary
        // transparent surface intercept the desktop.
        runtime.update_pointer_button(MouseButton::Left, ButtonState::Pressed, false);
        assert!(!runtime.pointer_locked);

        // Nor may a stale primary-window hit test lock a transparent location.
        runtime.update_pointer_button(MouseButton::Right, ButtonState::Pressed, false);
        assert!(!runtime.pointer_locked);

        runtime.update_pointer_button(MouseButton::Left, ButtonState::Pressed, true);
        assert!(runtime.pointer_locked);
        runtime.update_pointer_button(MouseButton::Left, ButtonState::Released, false);
        assert!(!runtime.pointer_locked);
    }

    #[test]
    fn pointer_lock_tracks_each_owned_button_until_release() {
        let mut runtime = OverlayWindowRuntime::default();
        runtime.update_pointer_button(MouseButton::Left, ButtonState::Pressed, true);
        runtime.update_pointer_button(MouseButton::Right, ButtonState::Pressed, true);

        runtime.update_pointer_button(MouseButton::Left, ButtonState::Released, false);
        assert!(runtime.pointer_locked);
        runtime.update_pointer_button(MouseButton::Right, ButtonState::Released, false);
        assert!(!runtime.pointer_locked);
    }

    #[test]
    fn an_external_file_drag_defers_native_window_resizing() {
        assert!(!should_defer_window_resize(false, false));
        assert!(should_defer_window_resize(true, false));
        assert!(should_defer_window_resize(false, true));
    }

    #[test]
    fn active_ole_drag_keeps_hit_testing_until_the_session_ends() {
        assert!(should_hit_test_overlay(false, false, true));
        assert!(!should_hit_test_overlay(false, false, false));
        assert!(should_hit_test_overlay(false, true, false));
    }

    #[test]
    fn ordinary_left_button_activity_cannot_start_an_external_drag() {
        // A normal click, text selection, or window move only changes the
        // physical button state. Without DRAGDROPSTART it must remain inactive.
        assert!(!transition_external_drag(
            false,
            ExternalDragTransition::PointerSample {
                left_button_down: true,
            },
        ));
        assert!(!transition_external_drag(
            false,
            ExternalDragTransition::PointerSample {
                left_button_down: false,
            },
        ));
    }

    #[test]
    fn external_drag_requires_start_and_clears_on_end_or_button_release() {
        let active = transition_external_drag(false, ExternalDragTransition::Start);
        assert!(active);
        assert!(transition_external_drag(
            active,
            ExternalDragTransition::PointerSample {
                left_button_down: true,
            },
        ));
        assert!(!transition_external_drag(
            active,
            ExternalDragTransition::End
        ));
        assert!(!transition_external_drag(
            active,
            ExternalDragTransition::PointerSample {
                left_button_down: false,
            },
        ));
    }

    #[test]
    fn native_cursor_sample_converts_physical_pixels_to_bevy_logical_units() {
        assert_eq!(
            logical_cursor_sample(Vec2::new(600.0, 300.0), Vec2::new(1_800.0, 1_400.0), 2.0),
            Some(PrimaryCursorSample {
                position: Vec2::new(300.0, 150.0),
                client_size: Vec2::new(900.0, 700.0),
            })
        );
    }

    #[test]
    fn native_cursor_sample_rejects_invalid_or_outside_positions() {
        let size = Vec2::new(1_800.0, 1_400.0);
        assert!(logical_cursor_sample(Vec2::new(-1.0, 300.0), size, 2.0).is_none());
        assert!(logical_cursor_sample(Vec2::new(1_800.0, 300.0), size, 2.0).is_none());
        assert!(logical_cursor_sample(Vec2::new(600.0, 300.0), size, 0.0).is_none());
    }

    #[test]
    fn composition_keeps_adjustable_lens_inside_safe_radius() {
        for lens_scale in [0.4, 1.0, 2.0] {
            let radius =
                CRITICAL_IMPACT_PARAMETER_RS * BACKGROUND_INFLUENCE_SHADOW_RADII * lens_scale
                    / (CAMERA_DISTANCE_RS * render_tan_half_fov(lens_scale));
            assert!(radius <= CONTENT_SAFE_RADIUS + 1.0e-5);
        }
    }

    #[test]
    fn lens_influence_contains_the_client_center() {
        let size = Vec2::new(1_600.0, 900.0);
        for lens_scale in [0.4, 1.0, 2.0] {
            assert!(cursor_hits_lens_influence(size * 0.5, size, lens_scale));
        }
    }

    #[test]
    fn lens_influence_accounts_for_client_aspect_ratio() {
        let wide = Vec2::new(1_600.0, 800.0);

        // Equal fractional offsets are not equal camera-space distances in a
        // wide client: the horizontal point is twice as far from the center.
        assert!(!cursor_hits_lens_influence(
            Vec2::new(wide.x * 0.75, wide.y * 0.5),
            wide,
            1.0,
        ));
        assert!(cursor_hits_lens_influence(
            Vec2::new(wide.x * 0.5, wide.y * 0.75),
            wide,
            1.0,
        ));

        // Conversely, equal pixel offsets along either axis land on the same
        // circular lens boundary after aspect correction.
        let radius_pixels = lens_influence_radius(1.0) * wide.y * 0.5;
        for offset in [Vec2::X, Vec2::Y] {
            assert!(cursor_hits_lens_influence(
                wide * 0.5 + offset * radius_pixels * 0.999,
                wide,
                1.0,
            ));
            assert!(!cursor_hits_lens_influence(
                wide * 0.5 + offset * radius_pixels * 1.001,
                wide,
                1.0,
            ));
        }
    }

    #[test]
    fn lens_influence_includes_its_exact_boundary_and_rejects_outside() {
        let size = Vec2::new(1_280.0, 720.0);
        let radius = lens_influence_radius(1.0);

        for direction in [Vec2::X, Vec2::Y, Vec2::new(0.6, 0.8)] {
            let boundary = cursor_for_screen_point(direction * radius, size);
            let inside = cursor_for_screen_point(direction * radius * 0.999, size);
            let outside = cursor_for_screen_point(direction * radius * 1.001, size);
            assert!(cursor_hits_lens_influence(boundary, size, 1.0));
            assert!(cursor_hits_lens_influence(inside, size, 1.0));
            assert!(!cursor_hits_lens_influence(outside, size, 1.0));
        }
    }

    #[test]
    fn lens_influence_rejects_zero_and_subpixel_client_sizes() {
        for size in [
            Vec2::ZERO,
            Vec2::splat(f32::MIN_POSITIVE),
            Vec2::new(1.0, 720.0),
            Vec2::new(1_280.0, 1.0),
            Vec2::new(-1.0, 720.0),
        ] {
            assert!(!cursor_hits_lens_influence(Vec2::ZERO, size, 1.0));
        }
    }

    #[test]
    fn lens_influence_clamps_finite_extremes_and_rejects_non_finite_inputs() {
        let size = Vec2::new(1_280.0, 720.0);
        let samples = [size * 0.5, size * Vec2::new(0.6, 0.5), Vec2::ZERO];

        for cursor in samples {
            assert_eq!(
                cursor_hits_lens_influence(cursor, size, -f32::MAX),
                cursor_hits_lens_influence(cursor, size, 0.4),
            );
            assert_eq!(
                cursor_hits_lens_influence(cursor, size, f32::MAX),
                cursor_hits_lens_influence(cursor, size, 2.0),
            );
        }

        for lens_scale in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
            assert!(!cursor_hits_lens_influence(size * 0.5, size, lens_scale));
        }
        assert!(!cursor_hits_lens_influence(
            Vec2::splat(f32::NAN),
            size,
            1.0,
        ));
        assert!(!cursor_hits_lens_influence(
            size * 0.5,
            Vec2::new(f32::INFINITY, size.y),
            1.0,
        ));
    }

    #[test]
    fn hit_region_passes_transparent_corners_and_keeps_disk() {
        let size = Vec2::new(1_125.0, 875.0);
        assert!(!cursor_hits_black_hole(Vec2::ZERO, size, 0.26, 1.0));
        assert!(cursor_hits_black_hole(size * 0.5, size, 0.26, 1.0));
        assert!(cursor_hits_black_hole(
            Vec2::new(size.x * 0.84, size.y * 0.5),
            size,
            0.26,
            1.0,
        ));
        assert!(!cursor_hits_black_hole(
            Vec2::new(size.x * 0.5, size.y * 0.03),
            size,
            0.26,
            1.0,
        ));
    }
}
