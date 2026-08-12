#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use sunk_core::{
    DropBatch, PerformanceController, VisualCaptureController, VisualCapturePhase,
    VisualCaptureSnapshot,
};
use sunk_desktop::{
    DesktopWindowConfig, DragBatchState, DragBatchUpdate, FileDropAggregator, NativeFileDropEvent,
};
use sunk_renderer::BlackHoleRenderer;
use sunk_settings::Settings;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

struct Application {
    window: Option<Arc<Window>>,
    renderer: Option<BlackHoleRenderer>,
    performance: PerformanceController,
    file_drop: FileDropAggregator,
    capture: VisualCaptureController,
    started_at: Instant,
    last_interaction: Instant,
    last_capture_update: Instant,
    last_quality_sample: Instant,
    next_frame_at: Instant,
}

impl Default for Application {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            window: None,
            renderer: None,
            performance: PerformanceController::default(),
            file_drop: FileDropAggregator::default(),
            capture: VisualCaptureController::default(),
            started_at: now,
            last_interaction: now,
            last_capture_update: now,
            last_quality_sample: now,
            next_frame_at: now,
        }
    }
}

impl Application {
    fn wake_rendering(&mut self) {
        self.next_frame_at = Instant::now();
    }

    fn mark_interaction(&mut self) {
        self.last_interaction = Instant::now();
        self.next_frame_at = self.last_interaction;
    }

    fn has_recent_interaction(&self) -> bool {
        self.last_interaction.elapsed().as_secs_f32() < 2.0
    }

    fn is_interactive(&self) -> bool {
        self.has_recent_interaction()
            || self.file_drop.state() != DragBatchState::Idle
            || self.capture.has_work()
    }

    fn handle_file_drop_event(&mut self, event: NativeFileDropEvent) {
        self.wake_rendering();
        match self.file_drop.push(event) {
            DragBatchUpdate::Rejected(reason) => {
                warn!(?reason, "rejected a structurally invalid file-drop event");
            }
            DragBatchUpdate::Cancelled => info!("file drag cancelled"),
            DragBatchUpdate::None
            | DragBatchUpdate::HoverStarted { .. }
            | DragBatchUpdate::HoverChanged { .. }
            | DragBatchUpdate::DropPending { .. } => {}
        }
    }

    fn submit_pending_drop(&mut self) {
        let Some(native_batch) = self.file_drop.flush_dropped() else {
            return;
        };
        let target_count = native_batch.len();
        let batch = match DropBatch::try_new(native_batch.into_paths()) {
            Ok(batch) => batch,
            Err(error) => {
                warn!(%error, target_count, "rejected an invalid file-drop batch");
                return;
            }
        };

        match self.capture.submit_batch(batch) {
            Ok(()) => info!(target_count, "queued a visual file-capture batch"),
            Err(error) => {
                warn!(%error, target_count, "could not queue a visual file-capture batch");
            }
        }
    }

    fn advance_capture(&mut self, now: Instant) {
        let delta = now
            .saturating_duration_since(self.last_capture_update)
            .min(Duration::from_millis(100));
        self.last_capture_update = now;
        if self.capture.has_work()
            && let Err(error) = self.capture.advance(delta)
        {
            error!(%error, "visual file-capture state failed");
            self.capture = VisualCaptureController::default();
        }
    }

    fn render_interaction(&self) -> f32 {
        let recent_input: f32 = if self.has_recent_interaction() {
            1.0
        } else {
            0.0
        };
        let hover: f32 = if self.file_drop.state() == DragBatchState::Hovering {
            0.72
        } else {
            0.0
        };
        let capture = self
            .capture
            .snapshots()
            .iter()
            .map(capture_visual_intensity)
            .fold(0.0_f32, f32::max);
        if self.capture.has_work() {
            capture
        } else {
            recent_input.max(hover)
        }
    }
}

fn capture_visual_intensity(snapshot: &VisualCaptureSnapshot) -> f32 {
    let progress = snapshot.progress.clamp(0.0, 1.0);
    match snapshot.phase {
        VisualCapturePhase::Attracting => 0.35 + progress * 0.25,
        VisualCapturePhase::Capturing => 0.60 + progress * 0.20,
        VisualCapturePhase::Orbiting => 0.80 + progress * 0.12,
        VisualCapturePhase::EnteringEventHorizon => 0.92 + progress * 0.08,
    }
}

impl ApplicationHandler for Application {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let settings = Settings::default();
        let window_config = DesktopWindowConfig::default();
        let window = match event_loop.create_window(window_config.attributes()) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                error!(%error, "failed to create the desktop window");
                event_loop.exit();
                return;
            }
        };

        let renderer = match pollster::block_on(BlackHoleRenderer::new(
            Arc::clone(&window),
            self.performance.current_quality(),
        )) {
            Ok(renderer) => renderer,
            Err(error) => {
                error!(%error, "failed to initialize the GPU renderer");
                event_loop.exit();
                return;
            }
        };

        info!(
            quality = ?self.performance.current_level(),
            permanent_delete = settings.file_operations.permanent_delete_enabled,
            "Sunk Phase 0 is running"
        );
        self.last_quality_sample = Instant::now();
        window.request_redraw();
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }

        match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(KeyCode::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
            }
            WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::MouseInput { .. } => self.mark_interaction(),
            WindowEvent::HoveredFile(path) => {
                self.handle_file_drop_event(NativeFileDropEvent::Hovered(path));
            }
            WindowEvent::HoveredFileCancelled => {
                self.handle_file_drop_event(NativeFileDropEvent::Cancelled);
            }
            WindowEvent::DroppedFile(path) => {
                self.handle_file_drop_event(NativeFileDropEvent::Dropped(path));
            }
            WindowEvent::RedrawRequested => {
                let interactive = self.is_interactive();
                let frame_started = Instant::now();
                let sample_period =
                    frame_started.saturating_duration_since(self.last_quality_sample);
                let elapsed = self.started_at.elapsed().as_secs_f32();
                let interaction = self.render_interaction();
                if let Some(renderer) = self.renderer.as_mut()
                    && let Err(error) = renderer.render(elapsed, interaction)
                {
                    error!(%error, "rendering failed");
                    event_loop.exit();
                    return;
                }

                let frame_time = frame_started.elapsed();
                self.last_quality_sample = frame_started;
                if let Some(quality) =
                    self.performance
                        .update(frame_time, sample_period, interactive)
                {
                    info!(quality = ?self.performance.current_level(), "render quality changed");
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.set_quality(quality);
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.advance_capture(now);
        self.submit_pending_drop();
        let interval = if self.is_interactive() {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(100)
        };
        if now >= self.next_frame_at {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
            self.next_frame_at = now + interval;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame_at));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,wgpu=warn")),
        )
        .with_target(false)
        .compact()
        .init();

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut Application::default())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(phase: VisualCapturePhase, progress: f32) -> VisualCaptureSnapshot {
        VisualCaptureSnapshot {
            visual_id: 1,
            phase,
            progress,
            orbit_lane: 0,
        }
    }

    #[test]
    fn capture_visual_intensity_is_bounded_and_phase_ordered() {
        let phases = [
            VisualCapturePhase::Attracting,
            VisualCapturePhase::Capturing,
            VisualCapturePhase::Orbiting,
            VisualCapturePhase::EnteringEventHorizon,
        ];
        let starts = phases.map(|phase| capture_visual_intensity(&snapshot(phase, 0.0)));
        let ends = phases.map(|phase| capture_visual_intensity(&snapshot(phase, 1.0)));

        for (actual, expected) in starts.into_iter().zip([0.35, 0.60, 0.80, 0.92]) {
            assert!((actual - expected).abs() < f32::EPSILON);
        }
        for (actual, expected) in ends.into_iter().zip([0.60, 0.80, 0.92, 1.0]) {
            assert!((actual - expected).abs() < f32::EPSILON);
        }
        assert!(capture_visual_intensity(&snapshot(phases[0], -1.0)) >= 0.0);
        assert!(capture_visual_intensity(&snapshot(phases[3], 2.0)) <= 1.0);
    }

    #[test]
    fn dropped_paths_are_submitted_once_without_filesystem_access() {
        let mut application = Application::default();
        application.handle_file_drop_event(NativeFileDropEvent::Dropped("virtual/a".into()));
        application.handle_file_drop_event(NativeFileDropEvent::Dropped("virtual/b".into()));

        application.submit_pending_drop();
        assert!(application.capture.has_work());
        assert_eq!(application.capture.snapshots().len(), 1);
        application
            .capture
            .advance(VisualCaptureController::TARGET_STAGGER)
            .unwrap();
        assert_eq!(application.capture.snapshots().len(), 2);

        application.submit_pending_drop();
        assert_eq!(application.capture.pending_batch_count(), 0);
    }

    #[test]
    fn cancelling_a_drag_never_starts_capture() {
        let mut application = Application::default();
        application.handle_file_drop_event(NativeFileDropEvent::Hovered("virtual/a".into()));
        application.handle_file_drop_event(NativeFileDropEvent::Cancelled);
        application.submit_pending_drop();

        assert!(!application.capture.has_work());
        assert_eq!(application.file_drop.state(), DragBatchState::Idle);
    }

    #[test]
    fn file_drop_does_not_replace_the_capture_timeline_with_pointer_highlight() {
        let mut application = Application::default();
        application.mark_interaction();
        application.handle_file_drop_event(NativeFileDropEvent::Dropped("virtual/a".into()));
        application.submit_pending_drop();

        let expected = capture_visual_intensity(&application.capture.snapshots()[0]);
        assert!((application.render_interaction() - expected).abs() < f32::EPSILON);
    }
}
