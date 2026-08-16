//! File drag-and-drop intake and the visual hand-off around the black hole.
//!
//! This module deliberately does not touch the filesystem. It accepts paths only
//! when Explorer releases them over the primary window's current black-hole hit
//! region, then hands the batch to a coordinator. The coordinator owns safety
//! checks, uninstall confirmation, and the eventual operation result.

use std::{
    collections::{HashSet, VecDeque},
    f32::consts::{PI, TAU},
    path::PathBuf,
};

use bevy::{
    camera::RenderTarget,
    prelude::*,
    window::{FileDragAndDrop, PrimaryWindow, WindowRef},
};

use crate::{
    black_hole::BlackHoleControls,
    settings::BlackHoleSettings,
    window_interaction::{OverlayWindowRuntime, primary_cursor_sample, render_tan_half_fov},
};

const MAX_DROP_BATCH_SIZE: usize = 256;
const VISUAL_QUEUE_INTERVAL_SECONDS: f64 = 0.120;
const ATTRACTING_SECONDS: f32 = 0.320;
const CAPTURING_SECONDS: f32 = 0.240;
const ORBITING_SECONDS: f32 = 0.960;
const EVENT_HORIZON_SECONDS: f32 = 0.320;
const SUCCESS_SECONDS: f32 = 0.440;
const FAILURE_SECONDS: f32 = 0.680;
const INVALID_DROP_PULSE_SECONDS: f32 = 0.520;
const VISUAL_Z: f32 = 100.0;

const CAMERA_DISTANCE_RS: f32 = 30.0;
const DISK_OUTER_RADIUS_RS: f32 = 11.5;
const CRITICAL_IMPACT_PARAMETER_RS: f32 = 2.598_076;
const DISK_LENSING_PADDING: f32 = 1.10;

/// Stable identifier assigned by the file-operation coordinator.
pub(crate) type DropVisualId = u64;

/// Ordered, de-duplicated paths released over the black hole in one frame.
#[derive(Message, Debug, Clone)]
pub(crate) struct DropBatchRequested {
    pub(crate) paths: Vec<PathBuf>,
    /// Drop point in the primary 2-D camera's world coordinates.
    pub(crate) drop_position: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropVisualKind {
    File,
    Application,
}

/// Commands sent back by the coordinator after classifying a dropped path.
#[derive(Message, Debug, Clone, Copy)]
pub(crate) enum VisualCommand {
    /// Starts a fully authorized animation. Ordinary files normally use this.
    Begin {
        id: DropVisualId,
        kind: DropVisualKind,
        start_position: Vec2,
    },
    /// Shows the attraction/capture feedback but holds before orbiting. This is
    /// useful while an application uninstall confirmation dialog is open.
    Stage {
        id: DropVisualId,
        kind: DropVisualKind,
        start_position: Vec2,
    },
    /// Releases a staged visual after validation or user confirmation.
    Authorize { id: DropVisualId },
    /// Reports the real operation result. Success is accepted only after
    /// `VisualOperationReady` has been emitted for the same identifier.
    Complete { id: DropVisualId, success: bool },
    /// Cancels validation or confirmation without performing an operation.
    Reject { id: DropVisualId },
}

/// Emitted once the visual reaches the event-horizon boundary. The visual waits
/// there until the coordinator reports `VisualCommand::Complete`.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisualOperationReady {
    pub(crate) id: DropVisualId,
}

/// Public scheduling points let the coordinator consume a batch and return
/// commands deterministically without depending on private system functions.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FileInteractionSystems {
    ObserveDrops,
    ReceiveCommands,
    Animate,
    Draw,
}

pub(crate) struct FileInteractionPlugin;

impl Plugin for FileInteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DropInteractionState>()
            .add_message::<DropBatchRequested>()
            .add_message::<VisualCommand>()
            .add_message::<VisualOperationReady>()
            .configure_sets(
                Update,
                (
                    FileInteractionSystems::ObserveDrops,
                    FileInteractionSystems::ReceiveCommands,
                    FileInteractionSystems::Animate,
                    FileInteractionSystems::Draw,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                observe_primary_file_drag.in_set(FileInteractionSystems::ObserveDrops),
            )
            .add_systems(
                Update,
                (receive_visual_commands, spawn_queued_visuals)
                    .chain()
                    .in_set(FileInteractionSystems::ReceiveCommands),
            )
            .add_systems(
                Update,
                animate_visuals.in_set(FileInteractionSystems::Animate),
            )
            .add_systems(
                Update,
                draw_interaction_feedback.in_set(FileInteractionSystems::Draw),
            );
    }
}

/// Read by the native window integration while an OLE drag is in progress.
/// Other systems should treat both public fields as read-only.
#[derive(Resource, Debug, Default)]
pub(crate) struct DropInteractionState {
    pub(crate) drag_active: bool,
    pub(crate) hovering_black_hole: bool,
    queued_visuals: VecDeque<QueuedVisual>,
    active_ids: HashSet<DropVisualId>,
    next_visual_spawn_at: f64,
    next_orbit_lane: u8,
    invalid_drop_pulse: Option<TransientPulse>,
}

#[derive(Debug)]
struct QueuedVisual {
    id: DropVisualId,
    kind: DropVisualKind,
    start_position: Vec2,
    authorized: bool,
}

#[derive(Debug, Clone, Copy)]
struct TransientPulse {
    position: Vec2,
    elapsed: f32,
}

#[derive(Component, Debug)]
struct DropVisual {
    id: DropVisualId,
    kind: DropVisualKind,
    phase: VisualPhase,
    elapsed: f32,
    start_position: Vec2,
    failure_origin: Vec2,
    orbit_lane: u8,
    authorized: bool,
    operation_ready_emitted: bool,
    operation_succeeded: bool,
}

#[derive(Component, Debug, Clone, Copy)]
struct DropVisualPart {
    base_rgb: [f32; 3],
    base_alpha: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisualPhase {
    Attracting,
    Capturing,
    Orbiting,
    EnteringEventHorizon,
    Success,
    Failure,
}

#[derive(Debug, Clone, Copy)]
struct VisualSample {
    position: Vec2,
    scale: f32,
    rotation: f32,
    alpha: f32,
}

#[allow(
    clippy::too_many_arguments,
    reason = "drop intake needs the native cursor, target camera, settings, and message endpoints"
)]
fn observe_primary_file_drag(
    mut events: MessageReader<FileDragAndDrop>,
    primary_window: Single<(Entity, &Window), With<PrimaryWindow>>,
    primary_cameras: Query<(&Camera, &RenderTarget, &GlobalTransform), With<Camera2d>>,
    controls: Res<BlackHoleControls>,
    settings: Res<BlackHoleSettings>,
    mut overlay_runtime: ResMut<OverlayWindowRuntime>,
    mut state: ResMut<DropInteractionState>,
    mut requests: MessageWriter<DropBatchRequested>,
) {
    let (primary_entity, window) = *primary_window;
    let cursor_sample = primary_cursor_sample(window, &mut overlay_runtime);
    let cursor_position = cursor_sample.map(|sample| sample.position);
    let cursor_over_target = cursor_sample.is_some_and(|sample| {
        drop_target_contains(
            sample.position,
            sample.client_size,
            controls.pitch(),
            settings.lens_radius,
        )
    });
    let drop_world_position = cursor_position.and_then(|cursor| {
        primary_cameras
            .iter()
            .find(|(_, target, _)| matches!(target, RenderTarget::Window(WindowRef::Primary)))
            .and_then(|(camera, _, transform)| camera.viewport_to_world_2d(transform, cursor).ok())
    });

    let mut hovered = false;
    let mut canceled = false;
    let mut dropped = Vec::new();

    for event in events.read() {
        match event {
            FileDragAndDrop::HoveredFile { window, .. } if *window == primary_entity => {
                hovered = true;
            }
            FileDragAndDrop::HoveredFileCanceled { window } if *window == primary_entity => {
                canceled = true;
            }
            FileDragAndDrop::DroppedFile { window, path_buf } if *window == primary_entity => {
                dropped.push(path_buf.clone());
            }
            // The settings window has its own native drop target. Its events
            // must never influence the transparent primary overlay.
            _ => {}
        }
    }

    if !dropped.is_empty() {
        let paths = collect_drop_batch(dropped);
        state.drag_active = false;
        state.hovering_black_hole = false;

        if cursor_over_target
            && let Some(drop_position) = drop_world_position
            && let Ok(paths) = paths
        {
            requests.write(DropBatchRequested {
                paths,
                drop_position,
            });
        } else {
            state.invalid_drop_pulse = Some(TransientPulse {
                position: drop_world_position.unwrap_or(Vec2::ZERO),
                elapsed: 0.0,
            });
        }
        return;
    }

    if canceled {
        state.drag_active = false;
        state.hovering_black_hole = false;
    } else {
        if hovered {
            state.drag_active = true;
        }
        state.hovering_black_hole = state.drag_active && cursor_over_target;
    }
}

fn receive_visual_commands(
    mut commands: MessageReader<VisualCommand>,
    mut state: ResMut<DropInteractionState>,
    mut visuals: Query<(&mut DropVisual, &Transform)>,
) {
    for command in commands.read().copied() {
        match command {
            VisualCommand::Begin {
                id,
                kind,
                start_position,
            } => queue_visual(&mut state, id, kind, start_position, true),
            VisualCommand::Stage {
                id,
                kind,
                start_position,
            } => queue_visual(&mut state, id, kind, start_position, false),
            VisualCommand::Authorize { id } => {
                if let Some(queued) = state
                    .queued_visuals
                    .iter_mut()
                    .find(|queued| queued.id == id)
                {
                    queued.authorized = true;
                    continue;
                }
                if let Some((mut visual, _)) =
                    visuals.iter_mut().find(|(visual, _)| visual.id == id)
                {
                    visual.authorized = true;
                }
            }
            VisualCommand::Complete { id, success } => {
                let Some((mut visual, transform)) =
                    visuals.iter_mut().find(|(visual, _)| visual.id == id)
                else {
                    continue;
                };

                if success {
                    // A result is trusted only after this module requested the
                    // operation. This prevents a stale response from skipping
                    // validation and playing a false-positive consume effect.
                    if visual.operation_ready_emitted
                        && visual.phase == VisualPhase::EnteringEventHorizon
                    {
                        visual.operation_succeeded = true;
                    }
                } else {
                    begin_failure(&mut visual, transform.translation.truncate());
                }
            }
            VisualCommand::Reject { id } => {
                if let Some(index) = state
                    .queued_visuals
                    .iter()
                    .position(|queued| queued.id == id)
                {
                    if let Some(queued) = state.queued_visuals.remove(index) {
                        state.active_ids.remove(&id);
                        state.invalid_drop_pulse = Some(TransientPulse {
                            position: queued.start_position,
                            elapsed: 0.0,
                        });
                    }
                    continue;
                }

                if let Some((mut visual, transform)) =
                    visuals.iter_mut().find(|(visual, _)| visual.id == id)
                {
                    begin_failure(&mut visual, transform.translation.truncate());
                }
            }
        }
    }
}

fn queue_visual(
    state: &mut DropInteractionState,
    id: DropVisualId,
    kind: DropVisualKind,
    start_position: Vec2,
    authorized: bool,
) {
    if !start_position.is_finite() || !state.active_ids.insert(id) {
        return;
    }
    state.queued_visuals.push_back(QueuedVisual {
        id,
        kind,
        start_position,
        authorized,
    });
}

fn spawn_queued_visuals(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<DropInteractionState>,
) {
    let now = time.elapsed_secs_f64();
    if now + f64::EPSILON < state.next_visual_spawn_at {
        return;
    }
    let Some(queued) = state.queued_visuals.pop_front() else {
        return;
    };

    let orbit_lane = state.next_orbit_lane;
    state.next_orbit_lane = (state.next_orbit_lane + 1) % 4;
    state.next_visual_spawn_at = now + VISUAL_QUEUE_INTERVAL_SECONDS;
    spawn_visual(&mut commands, queued, orbit_lane);
}

fn spawn_visual(commands: &mut Commands, queued: QueuedVisual, orbit_lane: u8) {
    commands
        .spawn((
            DropVisual {
                id: queued.id,
                kind: queued.kind,
                phase: VisualPhase::Attracting,
                elapsed: 0.0,
                start_position: queued.start_position,
                failure_origin: queued.start_position,
                orbit_lane,
                authorized: queued.authorized,
                operation_ready_emitted: false,
                operation_succeeded: false,
            },
            Transform::from_xyz(queued.start_position.x, queued.start_position.y, VISUAL_Z),
            Visibility::Visible,
        ))
        .with_children(|parent| match queued.kind {
            DropVisualKind::File => spawn_file_icon(parent),
            DropVisualKind::Application => spawn_application_icon(parent),
        });
}

fn spawn_file_icon(parent: &mut ChildSpawnerCommands) {
    spawn_icon_part(
        parent,
        Vec2::new(54.0, 60.0),
        Vec3::new(0.0, 0.0, 0.0),
        [0.50, 0.72, 1.0],
        0.16,
    );
    spawn_icon_part(
        parent,
        Vec2::new(38.0, 48.0),
        Vec3::new(0.0, 0.0, 0.1),
        [0.86, 0.94, 1.0],
        0.96,
    );
    spawn_icon_part(
        parent,
        Vec2::new(14.0, 10.0),
        Vec3::new(11.0, 17.0, 0.2),
        [1.0, 0.70, 0.34],
        1.0,
    );
    spawn_icon_part(
        parent,
        Vec2::new(23.0, 3.0),
        Vec3::new(-2.0, 2.0, 0.2),
        [0.28, 0.48, 0.70],
        0.72,
    );
    spawn_icon_part(
        parent,
        Vec2::new(18.0, 3.0),
        Vec3::new(-4.5, -7.0, 0.2),
        [0.28, 0.48, 0.70],
        0.58,
    );
}

fn spawn_application_icon(parent: &mut ChildSpawnerCommands) {
    spawn_icon_part(
        parent,
        Vec2::new(58.0, 58.0),
        Vec3::ZERO,
        [0.30, 0.96, 0.84],
        0.18,
    );
    spawn_icon_part(
        parent,
        Vec2::new(43.0, 43.0),
        Vec3::new(0.0, 0.0, 0.1),
        [0.16, 0.68, 0.74],
        0.98,
    );
    for (x, y) in [(-10.0, 10.0), (10.0, 10.0), (-10.0, -10.0), (10.0, -10.0)] {
        spawn_icon_part(
            parent,
            Vec2::splat(13.0),
            Vec3::new(x, y, 0.2),
            [0.84, 1.0, 0.92],
            0.90,
        );
    }
}

fn spawn_icon_part(
    parent: &mut ChildSpawnerCommands,
    size: Vec2,
    translation: Vec3,
    base_rgb: [f32; 3],
    base_alpha: f32,
) {
    parent.spawn((
        Sprite::from_color(
            Color::srgba(base_rgb[0], base_rgb[1], base_rgb[2], base_alpha),
            size,
        ),
        Transform::from_translation(translation),
        DropVisualPart {
            base_rgb,
            base_alpha,
        },
    ));
}

#[allow(
    clippy::too_many_arguments,
    reason = "the animation updates independent ECS views and its coordinator handshake"
)]
fn animate_visuals(
    mut commands: Commands,
    time: Res<Time>,
    primary_window: Single<&Window, With<PrimaryWindow>>,
    mut state: ResMut<DropInteractionState>,
    mut visuals: Query<(Entity, &mut DropVisual, &mut Transform, &Children)>,
    mut parts: Query<(&DropVisualPart, &mut Sprite), Without<DropVisual>>,
    mut operation_ready: MessageWriter<VisualOperationReady>,
) {
    let delta = time.delta_secs().min(0.1);
    let viewport_size = Vec2::new(primary_window.width(), primary_window.height());

    if let Some(pulse) = &mut state.invalid_drop_pulse {
        pulse.elapsed += delta;
        if pulse.elapsed >= INVALID_DROP_PULSE_SECONDS {
            state.invalid_drop_pulse = None;
        }
    }

    for (entity, mut visual, mut transform, children) in &mut visuals {
        let advance = advance_visual(&mut visual, delta);
        if let Some(id) = advance.operation_ready {
            operation_ready.write(VisualOperationReady { id });
        }
        let sample = visual_sample(
            visual.phase,
            visual.elapsed,
            visual.start_position,
            visual.failure_origin,
            visual.orbit_lane,
            viewport_size,
        );

        transform.translation = sample.position.extend(VISUAL_Z);
        transform.rotation = Quat::from_rotation_z(sample.rotation);
        transform.scale = Vec3::splat(sample.scale.max(0.001));

        let failure = visual.phase == VisualPhase::Failure;
        for child in children.iter() {
            if let Ok((part, mut sprite)) = parts.get_mut(child) {
                let rgb = if failure {
                    mix_rgb(part.base_rgb, [1.0, 0.12, 0.08], 0.82)
                } else {
                    part.base_rgb
                };
                sprite.color = Color::srgba(rgb[0], rgb[1], rgb[2], sample.alpha * part.base_alpha);
            }
        }

        if advance.finished {
            state.active_ids.remove(&visual.id);
            commands.entity(entity).despawn();
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct VisualAdvance {
    finished: bool,
    operation_ready: Option<DropVisualId>,
}

fn advance_visual(visual: &mut DropVisual, delta: f32) -> VisualAdvance {
    let mut advance = VisualAdvance::default();
    match visual.phase {
        VisualPhase::Attracting => {
            visual.elapsed += delta;
            if visual.elapsed >= ATTRACTING_SECONDS {
                visual.phase = VisualPhase::Capturing;
                visual.elapsed = 0.0;
            }
        }
        VisualPhase::Capturing => {
            visual.elapsed = (visual.elapsed + delta).min(CAPTURING_SECONDS);
            if visual.authorized && visual.elapsed >= CAPTURING_SECONDS {
                visual.phase = VisualPhase::Orbiting;
                visual.elapsed = 0.0;
            }
        }
        VisualPhase::Orbiting => {
            visual.elapsed += delta;
            if visual.elapsed >= ORBITING_SECONDS {
                visual.phase = VisualPhase::EnteringEventHorizon;
                visual.elapsed = 0.0;
                if !visual.operation_ready_emitted {
                    advance.operation_ready = Some(visual.id);
                    visual.operation_ready_emitted = true;
                }
            }
        }
        VisualPhase::EnteringEventHorizon => {
            // Hold visibly at the boundary until the real operation succeeds.
            if visual.operation_succeeded {
                visual.elapsed += delta;
                if visual.elapsed >= EVENT_HORIZON_SECONDS {
                    visual.phase = VisualPhase::Success;
                    visual.elapsed = 0.0;
                }
            }
        }
        VisualPhase::Success => {
            visual.elapsed += delta;
            advance.finished = visual.elapsed >= SUCCESS_SECONDS;
        }
        VisualPhase::Failure => {
            visual.elapsed += delta;
            advance.finished = visual.elapsed >= FAILURE_SECONDS;
        }
    }
    advance
}

fn begin_failure(visual: &mut DropVisual, origin: Vec2) {
    visual.phase = VisualPhase::Failure;
    visual.elapsed = 0.0;
    visual.failure_origin = origin;
    visual.operation_succeeded = false;
}

fn draw_interaction_feedback(
    time: Res<Time>,
    state: Res<DropInteractionState>,
    primary_window: Single<&Window, With<PrimaryWindow>>,
    visuals: Query<&DropVisual>,
    mut gizmos: Gizmos,
) {
    let viewport_size = Vec2::new(primary_window.width(), primary_window.height());
    let minimum_extent = viewport_size.min_element().max(1.0);

    if state.drag_active && state.hovering_black_hole {
        let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 5.2).sin();
        let radius = minimum_extent * (0.086 + 0.008 * pulse);
        let alpha = 0.34 + 0.36 * pulse;
        draw_dashed_ring(
            &mut gizmos,
            Vec2::ZERO,
            radius,
            time.elapsed_secs() * -0.72,
            Color::srgba(1.0, 0.72, 0.32, alpha),
        );
        gizmos
            .circle_2d(
                Vec2::ZERO,
                radius * 0.72,
                Color::srgba(0.82, 0.94, 1.0, alpha * 0.34),
            )
            .resolution(64);
    }

    if let Some(pulse) = state.invalid_drop_pulse {
        let t = normalized_time(pulse.elapsed, INVALID_DROP_PULSE_SECONDS);
        gizmos
            .circle_2d(
                pulse.position,
                18.0 + minimum_extent * 0.12 * ease_out_cubic(t),
                Color::srgba(1.0, 0.08, 0.05, (1.0 - t) * 0.78),
            )
            .resolution(48);
    }

    for visual in &visuals {
        match visual.phase {
            VisualPhase::Orbiting | VisualPhase::EnteringEventHorizon => {
                draw_orbit_trail(&mut gizmos, visual, viewport_size);
            }
            VisualPhase::Success => {
                let t = normalized_time(visual.elapsed, SUCCESS_SECONDS);
                let fade = 1.0 - smoothstep(0.18, 1.0, t);
                for (offset, strength) in [(0.0, 0.80), (0.045, 0.48), (0.09, 0.24)] {
                    gizmos
                        .circle_2d(
                            Vec2::ZERO,
                            minimum_extent * (0.035 + offset + 0.11 * ease_out_cubic(t)),
                            Color::srgba(1.0, 0.88, 0.62, fade * strength),
                        )
                        .resolution(64);
                }
            }
            VisualPhase::Failure => {
                let t = normalized_time(visual.elapsed, FAILURE_SECONDS);
                gizmos
                    .circle_2d(
                        visual.failure_origin,
                        16.0 + minimum_extent * 0.10 * ease_out_cubic(t),
                        Color::srgba(1.0, 0.05, 0.03, (1.0 - t) * 0.72),
                    )
                    .resolution(48);
            }
            VisualPhase::Attracting | VisualPhase::Capturing => {}
        }
    }
}

fn draw_dashed_ring(gizmos: &mut Gizmos, center: Vec2, radius: f32, rotation: f32, color: Color) {
    const SEGMENTS: usize = 20;
    const ARC_ANGLE: f32 = TAU / SEGMENTS as f32 * 0.58;
    for segment in 0..SEGMENTS {
        let angle = rotation + segment as f32 * TAU / SEGMENTS as f32;
        gizmos
            .arc_2d(
                Isometry2d::new(center, Rot2::radians(angle)),
                ARC_ANGLE,
                radius,
                color,
            )
            .resolution(4);
    }
}

fn draw_orbit_trail(gizmos: &mut Gizmos, visual: &DropVisual, viewport_size: Vec2) {
    let mut previous = visual_sample(
        visual.phase,
        visual.elapsed,
        visual.start_position,
        visual.failure_origin,
        visual.orbit_lane,
        viewport_size,
    )
    .position;
    for step in 1..=6 {
        let earlier = (visual.elapsed - step as f32 * 0.028).max(0.0);
        let point = visual_sample(
            visual.phase,
            earlier,
            visual.start_position,
            visual.failure_origin,
            visual.orbit_lane,
            viewport_size,
        )
        .position;
        let alpha = 0.18 * (1.0 - step as f32 / 7.0);
        let color = match visual.kind {
            DropVisualKind::File => Color::srgba(0.56, 0.78, 1.0, alpha),
            DropVisualKind::Application => Color::srgba(0.30, 1.0, 0.82, alpha),
        };
        gizmos.line_2d(previous, point, color);
        previous = point;
    }
}

fn visual_sample(
    phase: VisualPhase,
    elapsed: f32,
    start_position: Vec2,
    failure_origin: Vec2,
    orbit_lane: u8,
    viewport_size: Vec2,
) -> VisualSample {
    let minimum_extent = viewport_size.min_element().max(1.0);
    let lane = (orbit_lane % 4) as f32;
    let fallback_angle = lane * TAU / 4.0 + PI * 0.125;
    let start_angle = if start_position.length_squared() > 1.0 {
        start_position.y.atan2(start_position.x)
    } else {
        fallback_angle
    };
    let orbit_radius = minimum_extent * (0.305 - lane * 0.016);
    let horizon_radius = minimum_extent * 0.074;
    let capture_start_angle = start_angle - 0.18;
    let orbit_start_angle = start_angle - 0.56;
    let orbit_end_angle = orbit_start_angle - TAU * 1.62;

    match phase {
        VisualPhase::Attracting => {
            let t = normalized_time(elapsed, ATTRACTING_SECONDS);
            let eased = ease_out_cubic(t);
            let destination = orbit_point(capture_start_angle, orbit_radius * 1.22);
            VisualSample {
                position: start_position.lerp(destination, eased),
                scale: 1.0 - 0.08 * smoothstep(0.0, 1.0, t),
                rotation: -0.20 * eased,
                alpha: smoothstep(0.0, 0.18, t).max(0.36),
            }
        }
        VisualPhase::Capturing => {
            let t = normalized_time(elapsed, CAPTURING_SECONDS);
            let eased = smoothstep(0.0, 1.0, t);
            let angle = capture_start_angle + (orbit_start_angle - capture_start_angle) * eased;
            let radius = orbit_radius * (1.22 - 0.22 * eased);
            VisualSample {
                position: orbit_point(angle, radius),
                scale: 0.92 - 0.06 * eased,
                rotation: -0.20 - 0.48 * eased,
                alpha: 1.0,
            }
        }
        VisualPhase::Orbiting => {
            let t = normalized_time(elapsed, ORBITING_SECONDS);
            let accelerated = t * (0.78 + 0.22 * t);
            let angle = orbit_start_angle + (orbit_end_angle - orbit_start_angle) * accelerated;
            let radius = horizon_radius + (orbit_radius - horizon_radius) * (1.0 - t).powf(1.18);
            VisualSample {
                position: orbit_point(angle, radius),
                scale: 0.86 + (0.38 - 0.86) * smoothstep(0.0, 1.0, t),
                rotation: -0.68 - TAU * 1.95 * accelerated,
                alpha: 1.0 - 0.12 * smoothstep(0.70, 1.0, t),
            }
        }
        VisualPhase::EnteringEventHorizon => {
            let t = normalized_time(elapsed, EVENT_HORIZON_SECONDS);
            let eased = smoothstep(0.0, 1.0, t);
            let angle = orbit_end_angle - TAU * 0.58 * eased;
            let radius = horizon_radius * (1.0 - eased);
            VisualSample {
                position: orbit_point(angle, radius),
                scale: (0.38 * (1.0 - eased)).max(0.018),
                rotation: -0.68 - TAU * (1.95 + 0.58 * eased),
                alpha: 0.88 * (1.0 - smoothstep(0.08, 1.0, t)),
            }
        }
        VisualPhase::Success => VisualSample {
            position: Vec2::ZERO,
            scale: 0.018,
            rotation: 0.0,
            alpha: 0.0,
        },
        VisualPhase::Failure => {
            let t = normalized_time(elapsed, FAILURE_SECONDS);
            let direction = if failure_origin.length_squared() > 1.0 {
                failure_origin.normalize()
            } else {
                Vec2::from_angle(fallback_angle)
            };
            let rebound = minimum_extent * 0.24 * ease_out_back(t);
            VisualSample {
                position: failure_origin + direction * rebound,
                scale: 0.62 + 0.18 * (PI * t).sin(),
                rotation: 2.4
                    * t
                    * if orbit_lane.is_multiple_of(2) {
                        1.0
                    } else {
                        -1.0
                    },
                alpha: 1.0 - smoothstep(0.56, 1.0, t),
            }
        }
    }
}

fn orbit_point(angle: f32, radius: f32) -> Vec2 {
    // Clockwise angular motion matches the current WGSL disk advection when
    // viewed with the default camera. Vertical compression keeps the path on
    // the projected accretion plane instead of looking like a flat UI circle.
    Vec2::new(angle.cos() * radius, angle.sin() * radius * 0.43)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropBatchError {
    Empty,
    EmptyPath,
    TooManyTargets,
}

fn collect_drop_batch(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<PathBuf>, DropBatchError> {
    let mut unique = HashSet::new();
    let mut batch = Vec::with_capacity(MAX_DROP_BATCH_SIZE);
    for path in paths {
        if path.as_os_str().is_empty() {
            return Err(DropBatchError::EmptyPath);
        }
        if unique.insert(path.clone()) {
            batch.push(path);
            if batch.len() > MAX_DROP_BATCH_SIZE {
                return Err(DropBatchError::TooManyTargets);
            }
        }
    }
    if batch.is_empty() {
        Err(DropBatchError::Empty)
    } else {
        Ok(batch)
    }
}

fn drop_target_contains(cursor: Vec2, size: Vec2, pitch: f32, lens_radius: f32) -> bool {
    if !cursor.is_finite() || size.x <= 1.0 || size.y <= 1.0 {
        return false;
    }

    let aspect = size.x / size.y;
    let screen = Vec2::new(
        (cursor.x / size.x * 2.0 - 1.0) * aspect,
        1.0 - cursor.y / size.y * 2.0,
    );
    let tan_half_fov = render_tan_half_fov(lens_radius);
    let shadow_radius = CRITICAL_IMPACT_PARAMETER_RS / (CAMERA_DISTANCE_RS * tan_half_fov) * 1.18;
    if screen.length_squared() <= shadow_radius * shadow_radius {
        return true;
    }

    let disk_major = (DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING
        / (CAMERA_DISTANCE_RS * tan_half_fov))
        .min(0.98);
    let disk_minor = (disk_major * (pitch.abs().sin() + 0.10).min(1.0)).max(shadow_radius);
    (screen.x / disk_major).powi(2) + (screen.y / disk_minor).powi(2) <= 1.0
}

fn normalized_time(elapsed: f32, duration: f32) -> f32 {
    (elapsed / duration).clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease_out_cubic(value: f32) -> f32 {
    1.0 - (1.0 - value).powi(3)
}

fn ease_out_back(value: f32) -> f32 {
    const OVERSHOOT: f32 = 1.701_58;
    const SCALE: f32 = OVERSHOOT + 1.0;
    let shifted = value - 1.0;
    1.0 + SCALE * shifted.powi(3) + OVERSHOOT * shifted.powi(2)
}

fn mix_rgb(left: [f32; 3], right: [f32; 3], amount: f32) -> [f32; 3] {
    let amount = amount.clamp(0.0, 1.0);
    [
        left[0] + (right[0] - left[0]) * amount,
        left[1] + (right[1] - left[1]) * amount,
        left[2] + (right[2] - left[2]) * amount,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_preserves_order_and_deduplicates() {
        let paths = (0..200)
            .flat_map(|index| {
                let path = PathBuf::from(format!(r"C:\drop\item-{index}.dat"));
                [path.clone(), path]
            })
            .collect::<Vec<_>>();

        let batch = collect_drop_batch(paths).expect("valid batch");

        assert_eq!(batch.len(), 200);
        assert_eq!(batch[0], PathBuf::from(r"C:\drop\item-0.dat"));
        assert_eq!(batch[199], PathBuf::from(r"C:\drop\item-199.dat"));
    }

    #[test]
    fn oversized_or_empty_batches_are_rejected_as_a_batch() {
        let oversized = (0..=MAX_DROP_BATCH_SIZE)
            .map(|index| PathBuf::from(format!(r"C:\drop\item-{index}.dat")));
        assert_eq!(
            collect_drop_batch(oversized),
            Err(DropBatchError::TooManyTargets)
        );
        assert_eq!(collect_drop_batch([]), Err(DropBatchError::Empty));
        assert_eq!(
            collect_drop_batch([PathBuf::from(r"C:\drop\ok.dat"), PathBuf::new()]),
            Err(DropBatchError::EmptyPath)
        );
    }

    #[test]
    fn staged_visual_holds_after_capture_until_authorized() {
        let mut visual = test_visual(false);

        advance_visual(&mut visual, ATTRACTING_SECONDS);
        assert_eq!(visual.phase, VisualPhase::Capturing);
        advance_visual(&mut visual, CAPTURING_SECONDS * 2.0);
        assert_eq!(visual.phase, VisualPhase::Capturing);
        assert_eq!(visual.elapsed, CAPTURING_SECONDS);

        visual.authorized = true;
        advance_visual(&mut visual, 0.001);
        assert_eq!(visual.phase, VisualPhase::Orbiting);
    }

    #[test]
    fn visual_waits_at_horizon_until_success_is_reported() {
        let mut visual = test_visual(true);

        advance_visual(&mut visual, ATTRACTING_SECONDS);
        advance_visual(&mut visual, CAPTURING_SECONDS);
        let ready = advance_visual(&mut visual, ORBITING_SECONDS);
        assert_eq!(visual.phase, VisualPhase::EnteringEventHorizon);
        assert!(visual.operation_ready_emitted);
        assert_eq!(ready.operation_ready, Some(visual.id));

        advance_visual(&mut visual, EVENT_HORIZON_SECONDS * 2.0);
        assert_eq!(visual.phase, VisualPhase::EnteringEventHorizon);
        assert_eq!(visual.elapsed, 0.0);

        visual.operation_succeeded = true;
        advance_visual(&mut visual, EVENT_HORIZON_SECONDS);
        assert_eq!(visual.phase, VisualPhase::Success);
    }

    #[test]
    fn orbit_moves_clockwise_inward_and_entering_fades() {
        let size = Vec2::new(900.0, 700.0);
        let start = Vec2::new(330.0, 120.0);
        let orbit_start = visual_sample(VisualPhase::Orbiting, 0.0, start, start, 0, size);
        let orbit_end = visual_sample(
            VisualPhase::Orbiting,
            ORBITING_SECONDS,
            start,
            start,
            0,
            size,
        );
        let entering_end = visual_sample(
            VisualPhase::EnteringEventHorizon,
            EVENT_HORIZON_SECONDS,
            start,
            start,
            0,
            size,
        );

        assert!(orbit_end.position.length() < orbit_start.position.length());
        assert!(orbit_end.rotation < orbit_start.rotation);
        assert!(entering_end.position.length() < 0.01);
        assert_eq!(entering_end.alpha, 0.0);
    }

    #[test]
    fn drop_hit_test_accepts_the_hole_and_rejects_transparent_corners() {
        let size = Vec2::new(900.0, 700.0);
        assert!(drop_target_contains(size * 0.5, size, 0.26, 1.0));
        assert!(!drop_target_contains(Vec2::ZERO, size, 0.26, 1.0));
        assert!(!drop_target_contains(
            Vec2::new(size.x * 0.5, size.y * 0.02),
            size,
            0.26,
            1.0,
        ));
    }

    fn test_visual(authorized: bool) -> DropVisual {
        DropVisual {
            id: 7,
            kind: DropVisualKind::File,
            phase: VisualPhase::Attracting,
            elapsed: 0.0,
            start_position: Vec2::new(200.0, 100.0),
            failure_origin: Vec2::new(200.0, 100.0),
            orbit_lane: 0,
            authorized,
            operation_ready_emitted: false,
            operation_succeeded: false,
        }
    }
}
