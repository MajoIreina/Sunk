//! Stable boundary between the renderer and platform desktop capture.
//!
//! The first Windows backend intentionally favors proof over throughput: GDI
//! captures the small overlay rectangle on a worker thread and Bevy uploads the
//! latest frame. This validates self-exclusion, coordinate mapping, and physical
//! lensing before a D3D11/D3D12 shared-texture path is allowed to add complexity.

use bevy::prelude::*;

pub struct DesktopCapturePlugin;

impl Plugin for DesktopCapturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DesktopCaptureState>();

        #[cfg(target_os = "windows")]
        app.add_systems(PreStartup, windows_backend::start_capture)
            .add_systems(Update, windows_backend::upload_latest_frame);
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum DesktopCaptureMode {
    #[default]
    Disabled,
    WindowsGdiProof,
}

#[derive(Resource, Debug)]
pub struct DesktopCaptureState {
    pub mode: DesktopCaptureMode,
    /// GPU image containing the monitor pixels underneath this overlay.
    pub texture: Option<Handle<Image>>,
    /// Maps window-local UV into the captured desktop texture: origin.xy, scale.xy.
    pub uv_origin_scale: Vec4,
    /// Physical size of the overscanned capture texture, not the overlay window.
    pub frame_size: UVec2,
    pub frame_index: u64,
}

impl Default for DesktopCaptureState {
    fn default() -> Self {
        Self {
            mode: if cfg!(target_os = "windows") {
                DesktopCaptureMode::WindowsGdiProof
            } else {
                DesktopCaptureMode::Disabled
            },
            texture: None,
            uv_origin_scale: Vec4::new(0.0, 0.0, 1.0, 1.0),
            frame_size: UVec2::ZERO,
            frame_index: 0,
        }
    }
}

impl DesktopCaptureState {
    pub fn is_ready(&self) -> bool {
        self.mode != DesktopCaptureMode::Disabled
            && self.texture.is_some()
            && self.frame_size != UVec2::ZERO
            && self.frame_index > 0
    }
}

#[cfg(target_os = "windows")]
mod windows_backend {
    use std::{
        ffi::c_void,
        mem::size_of,
        ptr::null_mut,
        slice,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use bevy::{
        asset::RenderAssetUsages,
        image::ImageSampler,
        prelude::*,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
    use windows::{
        Win32::{
            Foundation::{HWND, POINT, RECT},
            Graphics::Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen,
                CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
                GdiFlush, GetDC, GetMonitorInfoW, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO,
                MonitorFromWindow, ROP_CODE, ReleaseDC, SRCCOPY, SelectObject,
            },
            UI::WindowsAndMessaging::{
                FindWindowW, GA_ROOT, GetAncestor, GetClientRect, GetWindowDisplayAffinity,
                GetWindowThreadProcessId, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
                WINDOW_DISPLAY_AFFINITY,
            },
        },
        core::PCWSTR,
    };

    use super::DesktopCaptureState;

    const WINDOW_TITLE: &str = "Sunk Black Hole";
    const CAPTURE_INTERVAL: Duration = Duration::from_millis(50);
    const ERROR_LOG_INTERVAL: Duration = Duration::from_secs(5);
    const OVERSCAN_RETRY_INTERVAL: Duration = Duration::from_secs(5);
    const MAX_CAPTURE_DIMENSION: i32 = 4_096;
    const OVERSCAN_FRACTION: f32 = 0.45;

    struct CapturedFrame {
        size: UVec2,
        rgba: Vec<u8>,
        uv_origin_scale: Vec4,
        index: u64,
    }

    #[derive(Clone, Copy, Debug)]
    struct ScreenRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    impl ScreenRect {
        fn width(self) -> i32 {
            self.right - self.left
        }

        fn height(self) -> i32 {
            self.bottom - self.top
        }

        fn size(self) -> IVec2 {
            IVec2::new(self.width(), self.height())
        }

        fn intersection(self, other: Self) -> Option<Self> {
            let intersection = Self {
                left: self.left.max(other.left),
                top: self.top.max(other.top),
                right: self.right.min(other.right),
                bottom: self.bottom.min(other.bottom),
            };
            (intersection.width() > 0 && intersection.height() > 0).then_some(intersection)
        }
    }

    #[derive(Clone, Copy)]
    struct ExcludedWindow {
        hwnd: HWND,
        previous_affinity: Option<u32>,
    }

    #[derive(Clone, Copy)]
    struct CaptureGeometry {
        client_rect: ScreenRect,
        capture_rect: ScreenRect,
        monitor_rect: ScreenRect,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CaptureRegionMode {
        Overscan,
        ClientOnly,
    }

    struct CapturePolicy {
        mode: CaptureRegionMode,
        retry_overscan_at: Instant,
    }

    impl CapturePolicy {
        fn new() -> Self {
            Self {
                mode: CaptureRegionMode::Overscan,
                retry_overscan_at: Instant::now(),
            }
        }
    }

    #[derive(Resource)]
    pub(super) struct CaptureBridge {
        latest: Arc<Mutex<Option<CapturedFrame>>>,
        running: Arc<AtomicBool>,
    }

    impl Drop for CaptureBridge {
        fn drop(&mut self) {
            self.running.store(false, Ordering::Release);
        }
    }

    pub(super) fn start_capture(
        mut commands: Commands,
        mut images: ResMut<Assets<Image>>,
        mut state: ResMut<DesktopCaptureState>,
    ) {
        let mut placeholder = Image::new_fill(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0, 0, 0],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        placeholder.sampler = ImageSampler::linear();
        state.texture = Some(images.add(placeholder));

        let latest = Arc::new(Mutex::new(None));
        let running = Arc::new(AtomicBool::new(true));
        let worker_latest = Arc::clone(&latest);
        let worker_running = Arc::clone(&running);

        thread::Builder::new()
            .name("sunk-desktop-capture".into())
            .spawn(move || capture_loop(worker_latest, worker_running))
            .expect("failed to start desktop capture worker");

        commands.insert_resource(CaptureBridge { latest, running });
    }

    pub(super) fn upload_latest_frame(
        bridge: Res<CaptureBridge>,
        mut state: ResMut<DesktopCaptureState>,
        mut images: ResMut<Assets<Image>>,
    ) {
        let frame = bridge
            .latest
            .lock()
            .ok()
            .and_then(|mut latest| latest.take());
        let Some(frame) = frame else {
            return;
        };
        let Some(handle) = state.texture.as_ref() else {
            return;
        };
        let Some(mut target) = images.get_mut(handle) else {
            return;
        };

        let mut image = Image::new(
            Extent3d {
                width: frame.size.x,
                height: frame.size.y,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            frame.rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::linear();
        *target = image;

        state.frame_size = frame.size;
        state.frame_index = frame.index;
        state.uv_origin_scale = frame.uv_origin_scale;
    }

    fn capture_loop(latest: Arc<Mutex<Option<CapturedFrame>>>, running: Arc<AtomicBool>) {
        let title = encode_window_title(WINDOW_TITLE);
        let mut frame_index = 0_u64;
        let hwnd = loop {
            if !running.load(Ordering::Acquire) {
                return;
            }
            if let Some(hwnd) = find_owned_window(&title) {
                break hwnd;
            }
            thread::sleep(CAPTURE_INTERVAL);
        };

        let mut excluded_windows = Vec::with_capacity(1);
        // Fail closed. Capturing without excluding the primary overlay would feed
        // it back into itself and quickly saturate the image.
        match exclude_window(hwnd, &mut excluded_windows) {
            Ok(Some(query_warning)) => warn!(
                "primary display-affinity query failed, but exclusion succeeded: {query_warning}"
            ),
            Ok(None) => {}
            Err(error) => {
                warn!("desktop capture disabled: self-exclusion failed: {error}");
                return;
            }
        }
        info!(
            "desktop capture proof enabled at 20 FPS (GDI CPU upload, {:.0}% overscan)",
            OVERSCAN_FRACTION * 100.0
        );
        // Do not apply display affinity to the opaque settings HWND. GDI BitBlt
        // returns ERROR_ACCESS_DENIED when its source rectangle intersects a second
        // WDA_EXCLUDEFROMCAPTURE window. The settings surface never samples this
        // texture, so allowing it into the background cannot form a feedback loop.
        info!("settings window remains capturable by the non-recursive GDI proof backend");

        let mut capture_failure = None;
        let mut capture_policy = CapturePolicy::new();
        while running.load(Ordering::Acquire) {
            let geometry = match capture_geometry(hwnd) {
                Ok(geometry) => geometry,
                Err(error) => {
                    report_capture_failure(&mut capture_failure, error);
                    thread::sleep(CAPTURE_INTERVAL);
                    continue;
                }
            };

            match capture_window(geometry, &mut capture_policy) {
                Ok((size, rgba, uv_origin_scale)) => {
                    if capture_failure.take().is_some() {
                        info!("desktop capture recovered");
                    }
                    frame_index += 1;
                    if let Ok(mut slot) = latest.lock() {
                        *slot = Some(CapturedFrame {
                            size,
                            rgba,
                            uv_origin_scale,
                            index: frame_index,
                        });
                    }
                }
                Err(error) => report_capture_failure(&mut capture_failure, error),
            }
            thread::sleep(CAPTURE_INTERVAL);
        }

        restore_window_affinities(&excluded_windows);
    }

    fn encode_window_title(title: &str) -> Vec<u16> {
        title.encode_utf16().chain(Some(0)).collect()
    }

    fn find_owned_window(title: &[u16]) -> Option<HWND> {
        // SAFETY: `title` is a NUL-terminated UTF-16 string for the duration of
        // this call. A null class name asks Windows to match by title.
        let hwnd = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }.ok()?;
        let mut owner_pid = 0_u32;
        // SAFETY: `owner_pid` is writable and `hwnd` is the title match above.
        let owner_thread =
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner_pid as *mut u32)) };
        (owner_thread != 0 && owner_pid == std::process::id()).then_some(hwnd)
    }

    fn exclude_window(
        hwnd: HWND,
        excluded_windows: &mut Vec<ExcludedWindow>,
    ) -> Result<Option<String>, String> {
        let hwnd = root_window(hwnd);
        if excluded_windows.iter().any(|entry| entry.hwnd == hwnd) {
            return Ok(None);
        }

        let mut previous_affinity = 0_u32;
        // Query and set are deliberately independent. Some DWM surface states can
        // reject the query while still accepting the exclusion operation.
        // SAFETY: `previous_affinity` is writable and `hwnd` belongs to this process.
        let query_result = unsafe { GetWindowDisplayAffinity(hwnd, &mut previous_affinity) };
        let query_warning = query_result
            .as_ref()
            .err()
            .map(|error| format!("GetWindowDisplayAffinity({hwnd:?}): {error}"));
        // SAFETY: Display affinity may be set for this process-owned top-level window.
        unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }.map_err(|error| {
            let query_context = query_warning
                .as_deref()
                .map(|warning| format!("; prior {warning}"))
                .unwrap_or_default();
            format!("SetWindowDisplayAffinity({hwnd:?}): {error}{query_context}")
        })?;
        excluded_windows.push(ExcludedWindow {
            hwnd,
            previous_affinity: query_result.is_ok().then_some(previous_affinity),
        });
        Ok(query_warning)
    }

    fn root_window(hwnd: HWND) -> HWND {
        // SAFETY: `hwnd` is a window found in this process. GA_ROOT walks only the
        // parent chain and returns null if the handle became invalid concurrently.
        let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
        if root.is_invalid() { hwnd } else { root }
    }

    fn report_capture_failure(state: &mut Option<(String, Instant)>, error: String) {
        let now = Instant::now();
        let should_report = state
            .as_ref()
            .is_none_or(|(previous, retry_at)| previous != &error || now >= *retry_at);
        if should_report {
            warn!("desktop capture frame skipped: {error}");
            *state = Some((error, now + ERROR_LOG_INTERVAL));
        }
    }

    fn restore_window_affinities(excluded_windows: &[ExcludedWindow]) {
        for entry in excluded_windows {
            // Best effort: the primary window may already be closing when this
            // worker exits.
            // SAFETY: Restores the value queried for this same process-owned HWND.
            let _ = unsafe {
                SetWindowDisplayAffinity(
                    entry.hwnd,
                    WINDOW_DISPLAY_AFFINITY(entry.previous_affinity.unwrap_or(0)),
                )
            };
        }
    }

    fn capture_geometry(hwnd: HWND) -> Result<CaptureGeometry, String> {
        let client_rect = client_rect_in_screen(hwnd)?;
        let monitor_rect = monitor_rect(hwnd)?;
        let capture_rect = overscan_rect(client_rect, monitor_rect)?;
        Ok(CaptureGeometry {
            client_rect,
            capture_rect,
            monitor_rect,
        })
    }

    fn capture_window(
        geometry: CaptureGeometry,
        policy: &mut CapturePolicy,
    ) -> Result<(UVec2, Vec<u8>, Vec4), String> {
        let now = Instant::now();
        let try_overscan =
            policy.mode == CaptureRegionMode::Overscan || now >= policy.retry_overscan_at;

        if try_overscan {
            match capture_source_rect(geometry.client_rect, geometry.capture_rect) {
                Ok(frame) => {
                    if policy.mode == CaptureRegionMode::ClientOnly {
                        info!("desktop capture restored monitor-local overscan");
                    }
                    policy.mode = CaptureRegionMode::Overscan;
                    policy.retry_overscan_at = now;
                    return Ok(frame);
                }
                Err(overscan_error) => {
                    let was_overscanned = policy.mode == CaptureRegionMode::Overscan;
                    policy.mode = CaptureRegionMode::ClientOnly;
                    policy.retry_overscan_at = now + OVERSCAN_RETRY_INTERVAL;
                    let client_capture = geometry
                        .client_rect
                        .intersection(geometry.monitor_rect)
                        .ok_or_else(|| {
                            format!(
                                "monitor-local overscan failed ({overscan_error}); client rectangle does not intersect its nearest monitor"
                            )
                        })?;
                    match capture_source_rect(geometry.client_rect, client_capture) {
                        Ok(frame) => {
                            if was_overscanned {
                                warn!(
                                    "monitor-local overscan failed; using client-only capture and retrying overscan every five seconds: {overscan_error}"
                                );
                            }
                            return Ok(frame);
                        }
                        Err(client_error) => {
                            return Err(format!(
                                "monitor-local overscan failed ({overscan_error}); client-only fallback failed ({client_error})"
                            ));
                        }
                    }
                }
            }
        }

        let client_capture = geometry
            .client_rect
            .intersection(geometry.monitor_rect)
            .ok_or("client rectangle does not intersect its nearest monitor")?;
        capture_source_rect(geometry.client_rect, client_capture)
    }

    fn capture_source_rect(
        client_rect: ScreenRect,
        capture_rect: ScreenRect,
    ) -> Result<(UVec2, Vec<u8>, Vec4), String> {
        // SAFETY: All GDI handles are checked before use, the DIB length exactly
        // matches width*height*4, and every acquired handle is released below.
        let (size, rgba) = unsafe {
            capture_gdi_rect(
                capture_rect.left,
                capture_rect.top,
                capture_rect.width(),
                capture_rect.height(),
            )
        }?;
        Ok((size, rgba, window_uv_mapping(client_rect, capture_rect)))
    }

    fn client_rect_in_screen(hwnd: HWND) -> Result<ScreenRect, String> {
        let mut client = RECT::default();
        // SAFETY: `client` is writable and `hwnd` remains valid while the app runs.
        unsafe { GetClientRect(hwnd, &mut client) }.map_err(|error| error.to_string())?;

        let mut top_left = POINT {
            x: client.left,
            y: client.top,
        };
        let mut bottom_right = POINT {
            x: client.right,
            y: client.bottom,
        };
        // SAFETY: Both points are writable client coordinates for this HWND.
        if !unsafe { ClientToScreen(hwnd, &mut top_left) }.as_bool()
            || !unsafe { ClientToScreen(hwnd, &mut bottom_right) }.as_bool()
        {
            return Err(windows::core::Error::from_thread().to_string());
        }

        let rect = ScreenRect {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        };
        if rect.width() <= 0 || rect.height() <= 0 {
            return Err(format!(
                "invalid client rectangle {}x{}",
                rect.width(),
                rect.height()
            ));
        }
        Ok(rect)
    }

    fn monitor_rect(hwnd: HWND) -> Result<ScreenRect, String> {
        // SAFETY: `hwnd` is the live primary window. The default-to-nearest flag
        // guarantees a monitor for windows moving across a display boundary.
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_invalid() {
            return Err("MonitorFromWindow returned an invalid monitor".into());
        }

        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..default()
        };
        // SAFETY: `info` has the required size and remains writable for the call.
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return Err(format!(
                "GetMonitorInfoW failed: {}",
                windows::core::Error::from_thread()
            ));
        }
        let rect = ScreenRect {
            left: info.rcMonitor.left,
            top: info.rcMonitor.top,
            right: info.rcMonitor.right,
            bottom: info.rcMonitor.bottom,
        };
        if rect.width() <= 0 || rect.height() <= 0 {
            return Err(format!(
                "invalid monitor rectangle ({}, {})-({}, {})",
                rect.left, rect.top, rect.right, rect.bottom
            ));
        }
        Ok(rect)
    }

    fn overscan_rect(client: ScreenRect, bounds: ScreenRect) -> Result<ScreenRect, String> {
        let client_size = client.size();
        if client_size.x > MAX_CAPTURE_DIMENSION || client_size.y > MAX_CAPTURE_DIMENSION {
            return Err(format!(
                "client rectangle {}x{} exceeds capture limit",
                client_size.x, client_size.y
            ));
        }

        let bounds_size = bounds.size();
        let desired_width = ((client_size.x as f32 * (1.0 + 2.0 * OVERSCAN_FRACTION)).ceil()
            as i32)
            .min(MAX_CAPTURE_DIMENSION)
            .min(bounds_size.x);
        let desired_height = ((client_size.y as f32 * (1.0 + 2.0 * OVERSCAN_FRACTION)).ceil()
            as i32)
            .min(MAX_CAPTURE_DIMENSION)
            .min(bounds_size.y);
        if desired_width <= 0 || desired_height <= 0 {
            return Err("overscan rectangle is empty".into());
        }

        // Keep a stable-sized capture near desktop edges by shifting the enlarged
        // rectangle into the selected monitor. This avoids reallocating the GPU image
        // merely because the overlay moved against an edge.
        let center_x = i64::from(client.left) + i64::from(client_size.x) / 2;
        let center_y = i64::from(client.top) + i64::from(client_size.y) / 2;
        let min_left = i64::from(bounds.left);
        let max_left = i64::from(bounds.right - desired_width);
        let min_top = i64::from(bounds.top);
        let max_top = i64::from(bounds.bottom - desired_height);
        let left = (center_x - i64::from(desired_width) / 2).clamp(min_left, max_left) as i32;
        let top = (center_y - i64::from(desired_height) / 2).clamp(min_top, max_top) as i32;

        Ok(ScreenRect {
            left,
            top,
            right: left + desired_width,
            bottom: top + desired_height,
        })
    }

    fn window_uv_mapping(client: ScreenRect, capture: ScreenRect) -> Vec4 {
        let capture_width = capture.width() as f32;
        let capture_height = capture.height() as f32;
        Vec4::new(
            (client.left - capture.left) as f32 / capture_width,
            (client.top - capture.top) as f32 / capture_height,
            client.width() as f32 / capture_width,
            client.height() as f32 / capture_height,
        )
    }

    unsafe fn capture_gdi_rect(
        left: i32,
        top: i32,
        width: i32,
        height: i32,
    ) -> Result<(UVec2, Vec<u8>), String> {
        // SAFETY: A null HWND requests the screen DC.
        let screen_dc = unsafe { GetDC(None) };
        if screen_dc.is_invalid() {
            return Err("GetDC returned an invalid screen DC".into());
        }

        // SAFETY: `screen_dc` is valid until ReleaseDC below.
        let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if memory_dc.is_invalid() {
            // SAFETY: Releases the screen DC acquired above.
            unsafe { ReleaseDC(None, screen_dc) };
            return Err("CreateCompatibleDC returned an invalid DC".into());
        }

        let byte_len = width as usize * height as usize * 4;
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // A negative height makes the DIB top-down, matching texture UV rows.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: byte_len as u32,
                ..default()
            },
            ..default()
        };
        let mut bits: *mut c_void = null_mut();
        // SAFETY: `bitmap_info` and `bits` remain valid through the call.
        let bitmap = match unsafe {
            CreateDIBSection(
                Some(screen_dc),
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            )
        } {
            Ok(bitmap) => bitmap,
            Err(error) => {
                // SAFETY: Releases both DCs acquired above.
                unsafe {
                    let _ = DeleteDC(memory_dc);
                    ReleaseDC(None, screen_dc);
                }
                return Err(error.to_string());
            }
        };

        // SAFETY: The bitmap is a valid GDI object compatible with `memory_dc`.
        let old_object = unsafe { SelectObject(memory_dc, HGDIOBJ::from(bitmap)) };
        let operation = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
        // SAFETY: Source/destination DCs and requested rectangle are valid.
        let copied = unsafe {
            BitBlt(
                memory_dc,
                0,
                0,
                width,
                height,
                Some(screen_dc),
                left,
                top,
                operation,
            )
        };

        // SAFETY: Flushes this thread's queued GDI operations before reading DIB memory.
        let _ = unsafe { GdiFlush() };

        let mut pixels = if copied.is_ok() && !bits.is_null() {
            // SAFETY: CreateDIBSection allocated at least `byte_len` bytes and it
            // remains selected/alive until after this copy.
            unsafe { slice::from_raw_parts(bits.cast::<u8>(), byte_len) }.to_vec()
        } else {
            Vec::new()
        };

        // Restore selection before deleting the bitmap, then release both DCs.
        // SAFETY: All handles were successfully acquired above.
        unsafe {
            if !old_object.is_invalid() {
                SelectObject(memory_dc, old_object);
            }
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
            let _ = DeleteDC(memory_dc);
            ReleaseDC(None, screen_dc);
        }

        copied.map_err(|error| {
            format!("BitBlt desktop copy failed for ({left}, {top}) {width}x{height}: {error}")
        })?;
        if pixels.len() != byte_len {
            return Err("CreateDIBSection returned no pixel storage".into());
        }

        // BI_RGB 32-bit DIBs are BGRA and leave alpha undefined.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }

        Ok((UVec2::new(width as u32, height as u32), pixels))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn rect(left: i32, top: i32, width: i32, height: i32) -> ScreenRect {
            ScreenRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            }
        }

        fn assert_close(actual: f32, expected: f32) {
            assert!((actual - expected).abs() < 1.0e-3, "{actual} != {expected}");
        }

        #[test]
        fn overscan_shifts_inside_negative_monitor_coordinates() {
            let client = rect(-1_800, 100, 900, 700);
            let desktop = rect(-1_920, 0, 3_840, 2_160);
            let capture = overscan_rect(client, desktop).expect("valid overscan rectangle");

            assert_eq!(capture.width(), 1_710);
            assert_eq!(capture.height(), 1_330);
            assert_eq!(capture.left, desktop.left);
            assert_eq!(capture.top, desktop.top);
            assert!(capture.right <= desktop.right);
            assert!(capture.bottom <= desktop.bottom);
        }

        #[test]
        fn uv_mapping_round_trips_window_corners() {
            let client = rect(-1_800, 100, 900, 700);
            let capture = rect(-1_920, 0, 1_710, 1_330);
            let mapping = window_uv_mapping(client, capture);

            assert_close(
                capture.left as f32 + mapping.x * capture.width() as f32,
                client.left as f32,
            );
            assert_close(
                capture.top as f32 + mapping.y * capture.height() as f32,
                client.top as f32,
            );
            assert_close(
                capture.left as f32 + (mapping.x + mapping.z) * capture.width() as f32,
                client.right as f32,
            );
            assert_close(
                capture.top as f32 + (mapping.y + mapping.w) * capture.height() as f32,
                client.bottom as f32,
            );
        }
    }
}
