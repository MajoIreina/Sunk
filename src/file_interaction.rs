//! File drag-and-drop intake and the visual hand-off into the black hole.
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
    settings::{BlackHoleSettings, clamped_lens_influence_scale},
    window_interaction::{OverlayWindowRuntime, primary_cursor_sample, render_tan_half_fov},
};

const MAX_DROP_BATCH_SIZE: usize = 256;
const VISUAL_QUEUE_INTERVAL_SECONDS: f64 = 0.120;
const ATTRACTING_SECONDS: f32 = 0.260;
const CAPTURING_SECONDS: f32 = 0.180;
const INFALLING_SECONDS: f32 = 0.720;
const EVENT_HORIZON_SECONDS: f32 = 0.240;
const SUCCESS_SECONDS: f32 = 0.120;
const FAILURE_SECONDS: f32 = 0.680;
const INVALID_DROP_PULSE_SECONDS: f32 = 0.520;
const VISUAL_Z: f32 = 100.0;
#[cfg(test)]
const ICON_MAX_HALF_DIAGONAL: f32 = 42.0;

const CAMERA_DISTANCE_RS: f32 = 30.0;
const DISK_OUTER_RADIUS_RS: f32 = 11.5;
const CRITICAL_IMPACT_PARAMETER_RS: f32 = 2.598_076;
const BACKGROUND_INFLUENCE_SHADOW_RADII: f32 = 3.45;
const DISK_LENSING_PADDING: f32 = 1.10;
const SHADOW_INTERACTION_PADDING: f32 = 1.18;

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
    /// Shows the attraction/capture feedback but holds before infall. This is
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
    /// Window-local texture coordinate of the native OLE cursor.
    pub(crate) feedback_uv: Vec2,
    /// Physical lens falloff at the cursor, zero outside the influence range.
    pub(crate) influence_strength: f32,
    queued_visuals: VecDeque<QueuedVisual>,
    active_ids: HashSet<DropVisualId>,
    next_visual_spawn_at: f64,
    next_infall_lane: u8,
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
    failure: FailureVisualState,
    infall_lane: u8,
    authorized: bool,
    operation_ready_emitted: bool,
    operation_succeeded: bool,
}

#[derive(Debug, Clone, Copy)]
struct FailureVisualState {
    origin: Vec2,
    radial_scale: f32,
    tangential_scale: f32,
    alpha: f32,
}

#[derive(Component, Debug, Clone, Copy)]
struct DropVisualPart {
    size: Vec2,
    local_translation: Vec3,
    base_rgb: [f32; 3],
    base_alpha: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisualPhase {
    Attracting,
    Capturing,
    Infalling,
    EnteringEventHorizon,
    Success,
    Failure,
}

#[derive(Debug, Clone, Copy)]
struct VisualSample {
    position: Vec2,
    radial_direction: Vec2,
    radial_scale: f32,
    tangential_scale: f32,
    alpha: f32,
    redshift: f32,
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
    let cursor_feedback = cursor_sample.map(|sample| {
        (
            sample.position / sample.client_size,
            drag_lens_influence_strength(sample.position, sample.client_size, settings.lens_radius),
        )
    });
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
        state.influence_strength = 0.0;

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
        state.influence_strength = 0.0;
    } else {
        if hovered {
            state.drag_active = true;
        }
        state.hovering_black_hole = state.drag_active && cursor_over_target;
        if state.drag_active {
            if let Some((feedback_uv, influence_strength)) = cursor_feedback {
                state.feedback_uv = feedback_uv;
                state.influence_strength = influence_strength;
            } else {
                state.influence_strength = 0.0;
            }
        } else {
            state.influence_strength = 0.0;
        }
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
                    if can_accept_operation_success(&visual) {
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

fn can_accept_operation_success(visual: &DropVisual) -> bool {
    visual.operation_ready_emitted && visual.phase == VisualPhase::EnteringEventHorizon
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

    let infall_lane = state.next_infall_lane;
    state.next_infall_lane = (state.next_infall_lane + 1) % 4;
    state.next_visual_spawn_at = now + VISUAL_QUEUE_INTERVAL_SECONDS;
    spawn_visual(&mut commands, queued, infall_lane);
}

fn spawn_visual(commands: &mut Commands, queued: QueuedVisual, infall_lane: u8) {
    commands
        .spawn((
            DropVisual {
                id: queued.id,
                kind: queued.kind,
                phase: VisualPhase::Attracting,
                elapsed: 0.0,
                start_position: queued.start_position,
                failure: FailureVisualState {
                    origin: queued.start_position,
                    radial_scale: 1.0,
                    tangential_scale: 1.0,
                    alpha: 1.0,
                },
                infall_lane,
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
    spawn_icon_grid(
        parent,
        Vec2::new(38.0, 48.0),
        Vec3::new(0.0, 0.0, 0.1),
        [0.86, 0.94, 1.0],
        0.96,
        UVec2::splat(4),
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
    spawn_icon_grid(
        parent,
        Vec2::new(43.0, 43.0),
        Vec3::new(0.0, 0.0, 0.1),
        [0.16, 0.68, 0.74],
        0.98,
        UVec2::splat(4),
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
            size,
            local_translation: translation,
            base_rgb,
            base_alpha,
        },
    ));
}

fn spawn_icon_grid(
    parent: &mut ChildSpawnerCommands,
    size: Vec2,
    translation: Vec3,
    base_rgb: [f32; 3],
    base_alpha: f32,
    subdivisions: UVec2,
) {
    let subdivisions = subdivisions.max(UVec2::ONE);
    let cell_size = size / subdivisions.as_vec2();
    let lower_left = -size * 0.5 + cell_size * 0.5;
    for y in 0..subdivisions.y {
        for x in 0..subdivisions.x {
            let cell_offset = lower_left + Vec2::new(x as f32, y as f32) * cell_size;
            spawn_icon_part(
                parent,
                cell_size,
                translation + cell_offset.extend(0.0),
                base_rgb,
                base_alpha,
            );
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the animation updates independent ECS views and its coordinator handshake"
)]
fn animate_visuals(
    mut commands: Commands,
    time: Res<Time>,
    primary_window: Single<&Window, With<PrimaryWindow>>,
    settings: Res<BlackHoleSettings>,
    mut state: ResMut<DropInteractionState>,
    mut visuals: Query<(Entity, &mut DropVisual, &mut Transform, &Children)>,
    mut parts: Query<(&DropVisualPart, &mut Transform, &mut Sprite), Without<DropVisual>>,
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
            visual.failure,
            visual.kind,
            visual.infall_lane,
            viewport_size,
            settings.lens_radius,
        );

        transform.translation = sample.position.extend(VISUAL_Z);
        let radial_angle = sample.radial_direction.y.atan2(sample.radial_direction.x);
        transform.rotation = Quat::from_rotation_z(radial_angle);
        transform.scale = Vec3::new(
            sample.radial_scale.max(0.001),
            sample.tangential_scale.max(0.001),
            1.0,
        );

        let failure = visual.phase == VisualPhase::Failure;
        if !failure {
            visual.failure.radial_scale = sample.radial_scale;
            visual.failure.tangential_scale = sample.tangential_scale;
            visual.failure.alpha = sample.alpha;
        }
        let shadow_radius = rendered_shadow_radius(viewport_size, settings.lens_radius);
        for child in children.iter() {
            if let Ok((part, mut child_transform, mut sprite)) = parts.get_mut(child) {
                // The parent axes follow the radial/tangential field. Counter-rotate
                // each part so an isotropic sample preserves the original icon
                // orientation while anisotropic samples produce tidal shear only.
                let local_xy = rotate_2d(part.local_translation.truncate(), -radial_angle);
                child_transform.translation = local_xy.extend(part.local_translation.z);
                child_transform.rotation = Quat::from_rotation_z(-radial_angle);
                child_transform.scale = Vec3::ONE;

                let rgb = if failure {
                    mix_rgb(part.base_rgb, [1.0, 0.12, 0.08], 0.82)
                } else {
                    apply_gravitational_redshift(part.base_rgb, sample.redshift)
                };
                let geometric_visibility = shadow_visibility(
                    sample.position,
                    sample.radial_direction,
                    sample.radial_scale,
                    part.local_translation.truncate(),
                    part.size,
                    shadow_radius,
                );
                sprite.color = Color::srgba(
                    rgb[0],
                    rgb[1],
                    rgb[2],
                    sample.alpha * geometric_visibility * part.base_alpha,
                );
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
    let mut remaining = if delta.is_finite() {
        delta.max(0.0)
    } else {
        0.0
    };

    for _ in 0..6 {
        match visual.phase {
            VisualPhase::Attracting => {
                if !consume_phase_time(&mut visual.elapsed, ATTRACTING_SECONDS, &mut remaining) {
                    break;
                }
                visual.phase = VisualPhase::Capturing;
            }
            VisualPhase::Capturing => {
                if !visual.authorized {
                    visual.elapsed = (visual.elapsed + remaining).min(CAPTURING_SECONDS);
                    break;
                }
                if !consume_phase_time(&mut visual.elapsed, CAPTURING_SECONDS, &mut remaining) {
                    break;
                }
                visual.phase = VisualPhase::Infalling;
            }
            VisualPhase::Infalling => {
                if !consume_phase_time(&mut visual.elapsed, INFALLING_SECONDS, &mut remaining) {
                    break;
                }
                visual.phase = VisualPhase::EnteringEventHorizon;
                if !visual.operation_ready_emitted {
                    advance.operation_ready = Some(visual.id);
                    visual.operation_ready_emitted = true;
                }
                // The operation cannot succeed before the ready message is
                // observed, so excess frame time stops at the visible boundary.
                break;
            }
            VisualPhase::EnteringEventHorizon => {
                // Hold visibly at the boundary until the real operation succeeds.
                if !visual.operation_succeeded {
                    break;
                }
                if !consume_phase_time(&mut visual.elapsed, EVENT_HORIZON_SECONDS, &mut remaining) {
                    break;
                }
                visual.phase = VisualPhase::Success;
            }
            VisualPhase::Success => {
                advance.finished =
                    consume_phase_time(&mut visual.elapsed, SUCCESS_SECONDS, &mut remaining);
                break;
            }
            VisualPhase::Failure => {
                advance.finished =
                    consume_phase_time(&mut visual.elapsed, FAILURE_SECONDS, &mut remaining);
                break;
            }
        }
    }
    advance
}

fn consume_phase_time(elapsed: &mut f32, duration: f32, remaining: &mut f32) -> bool {
    let until_end = (duration - *elapsed).max(0.0);
    if *remaining + f32::EPSILON < until_end {
        *elapsed += *remaining;
        *remaining = 0.0;
        false
    } else {
        *remaining = (*remaining - until_end).max(0.0);
        *elapsed = 0.0;
        true
    }
}

fn begin_failure(visual: &mut DropVisual, origin: Vec2) {
    visual.phase = VisualPhase::Failure;
    visual.elapsed = 0.0;
    visual.failure.origin = origin;
    visual.operation_succeeded = false;
}

fn draw_interaction_feedback(
    time: Res<Time>,
    state: Res<DropInteractionState>,
    primary_window: Single<&Window, With<PrimaryWindow>>,
    settings: Res<BlackHoleSettings>,
    visuals: Query<&DropVisual>,
    mut gizmos: Gizmos,
) {
    let viewport_size = Vec2::new(primary_window.width(), primary_window.height());
    let minimum_extent = viewport_size.min_element().max(1.0);
    let horizon_radius = rendered_shadow_radius(viewport_size, settings.lens_radius);

    if state.drag_active && state.hovering_black_hole {
        let contraction = (time.elapsed_secs() * 1.45).fract();
        for offset in [0.0, 0.5] {
            let t = (contraction + offset).fract();
            let radius = horizon_radius * (1.82 - 0.72 * ease_out_cubic(t));
            let alpha = (PI * t).sin().powi(2) * 0.22;
            gizmos
                .circle_2d(Vec2::ZERO, radius, Color::srgba(1.0, 0.62, 0.22, alpha))
                .resolution(64);
        }
        gizmos
            .circle_2d(
                Vec2::ZERO,
                horizon_radius,
                Color::srgba(0.94, 0.80, 0.58, 0.24),
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
            VisualPhase::Infalling => {
                draw_infall_trail(&mut gizmos, visual, viewport_size, settings.lens_radius);
            }
            VisualPhase::Success => {
                let t = normalized_time(visual.elapsed, SUCCESS_SECONDS);
                let fade = 1.0 - smoothstep(0.0, 1.0, t);
                gizmos
                    .circle_2d(
                        Vec2::ZERO,
                        horizon_radius * (1.02 - 0.10 * ease_out_cubic(t)),
                        Color::srgba(0.92, 0.30, 0.06, fade * 0.14),
                    )
                    .resolution(64);
            }
            VisualPhase::Failure => {
                let t = normalized_time(visual.elapsed, FAILURE_SECONDS);
                gizmos
                    .circle_2d(
                        visual.failure.origin,
                        16.0 + minimum_extent * 0.10 * ease_out_cubic(t),
                        Color::srgba(1.0, 0.05, 0.03, (1.0 - t) * 0.72),
                    )
                    .resolution(48);
            }
            VisualPhase::Attracting
            | VisualPhase::Capturing
            | VisualPhase::EnteringEventHorizon => {}
        }
    }
}

fn draw_infall_trail(
    gizmos: &mut Gizmos,
    visual: &DropVisual,
    viewport_size: Vec2,
    lens_radius: f32,
) {
    let current = visual_sample(
        visual.phase,
        visual.elapsed,
        visual.start_position,
        visual.failure,
        visual.kind,
        visual.infall_lane,
        viewport_size,
        lens_radius,
    );
    if current.position.length() <= rendered_shadow_radius(viewport_size, lens_radius) {
        return;
    }
    let mut previous = current.position;
    for step in 1..=8 {
        let earlier = (visual.elapsed - step as f32 * 0.024).max(0.0);
        let earlier_sample = visual_sample(
            visual.phase,
            earlier,
            visual.start_position,
            visual.failure,
            visual.kind,
            visual.infall_lane,
            viewport_size,
            lens_radius,
        );
        let alpha = current.alpha * 0.28 * (1.0 - step as f32 / 9.0).powi(2);
        let base_rgb = match visual.kind {
            DropVisualKind::File => [0.56, 0.78, 1.0],
            DropVisualKind::Application => [0.30, 1.0, 0.82],
        };
        let rgb = apply_gravitational_redshift(base_rgb, current.redshift);
        gizmos.line_2d(
            previous,
            earlier_sample.position,
            Color::srgba(rgb[0], rgb[1], rgb[2], alpha),
        );
        previous = earlier_sample.position;
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the pure sampler combines lifecycle, icon geometry, and viewport projection inputs"
)]
fn visual_sample(
    phase: VisualPhase,
    elapsed: f32,
    start_position: Vec2,
    failure: FailureVisualState,
    kind: DropVisualKind,
    infall_lane: u8,
    viewport_size: Vec2,
    lens_radius: f32,
) -> VisualSample {
    let minimum_extent = viewport_size.min_element().max(1.0);
    let lane = (infall_lane % 4) as f32;
    let fallback_angle = lane * TAU / 4.0 + PI * 0.125;
    let release_radius = start_position.length();
    let direction = if release_radius > f32::EPSILON {
        start_position / release_radius
    } else {
        Vec2::from_angle(fallback_angle)
    };
    let icon_half_extent = icon_radial_half_extent(kind, direction);
    // The ray-traced shadow hides the geometric event horizon. Its critical-impact
    // boundary drives visual occlusion; the wider pointer hit tolerance stays out
    // of this geometry so the icon meets the shadow actually shown on screen.
    let shadow_radius = rendered_shadow_radius(viewport_size, lens_radius);
    let fully_visible_radius = shadow_radius + icon_half_extent;
    let contact_ratio = (release_radius / fully_visible_radius.max(1.0)).clamp(0.0, 1.0);
    // Preserve radial size while compressing the tangential axis. The icon is
    // swallowed by the shadow mask instead of uniformly shrinking into a point.
    let boundary_base_scale = 0.50 + 0.08 * contact_ratio;
    let (boundary_radial_scale, boundary_tangential_scale) = tidal_scales(boundary_base_scale, 1.0);
    let boundary_alpha = 0.24 + 0.76 * contact_ratio;
    let boundary_redshift = 0.96 - 0.10 * contact_ratio;

    let initial_edge_gap = release_radius - fully_visible_radius;
    let outer_edge_gap = initial_edge_gap.max(0.0);
    let overlap_edge_gap = initial_edge_gap.min(0.0);
    // Keep the first sample at the real release point. Fully external icons
    // approach without crossing the shadow; icons already overlapping it keep
    // moving inward and are clipped against the real rendered boundary.
    let start_radius = release_radius;
    let attraction_end_radius =
        (shadow_radius + icon_half_extent * 0.94 + overlap_edge_gap + outer_edge_gap * 0.76)
            .max(0.0);
    let capture_end_radius =
        (shadow_radius + icon_half_extent * 0.88 + overlap_edge_gap + outer_edge_gap * 0.58)
            .max(0.0);
    let occlusion_radius =
        (shadow_radius + icon_half_extent * boundary_radial_scale + overlap_edge_gap).max(0.0);
    let (crossing_radial_scale, crossing_tangential_scale) =
        tidal_scales(boundary_base_scale * 0.88, 1.18);
    let crossing_end_radius = (shadow_radius - icon_half_extent * crossing_radial_scale - 2.0)
        .clamp(0.0, occlusion_radius);
    let attraction_span = (start_radius - attraction_end_radius).max(0.0);
    let capture_span = (attraction_end_radius - capture_end_radius).max(0.0);
    let capture_start_slope = if capture_span > f32::EPSILON {
        (2.0 * attraction_span * CAPTURING_SECONDS / (ATTRACTING_SECONDS * capture_span))
            .clamp(0.0, 3.0)
    } else {
        0.0
    };
    let attraction_end_slope = if attraction_span > f32::EPSILON {
        capture_span * capture_start_slope * ATTRACTING_SECONDS
            / (attraction_span * CAPTURING_SECONDS)
    } else {
        0.0
    };
    let capture_scale_start_slope = attraction_end_slope * CAPTURING_SECONDS / ATTRACTING_SECONDS;

    match phase {
        VisualPhase::Attracting => {
            let t = normalized_time(elapsed, ATTRACTING_SECONDS);
            let accelerated = cubic_hermite_progress(t, 0.0, attraction_end_slope);
            let radius = start_radius + (attraction_end_radius - start_radius) * accelerated;
            let scale = 1.0 - 0.06 * accelerated;
            VisualSample {
                position: direction * radius,
                radial_direction: direction,
                radial_scale: scale,
                tangential_scale: scale,
                alpha: 1.0,
                redshift: 0.0,
            }
        }
        VisualPhase::Capturing => {
            let t = normalized_time(elapsed, CAPTURING_SECONDS);
            // Match the incoming velocity of the attraction segment and
            // settle to zero speed for either a confirmation hold or infall.
            let eased = cubic_hermite_progress(t, capture_start_slope, 0.0);
            let scale_eased = cubic_hermite_progress(t, capture_scale_start_slope, 0.0);
            let radius =
                attraction_end_radius + (capture_end_radius - attraction_end_radius) * eased;
            let base_scale = 0.94 - 0.06 * scale_eased;
            let (radial_scale, tangential_scale) =
                tidal_scales(base_scale, 0.10 * smoothstep(0.0, 1.0, t));
            VisualSample {
                position: direction * radius,
                radial_direction: direction,
                radial_scale,
                tangential_scale,
                alpha: 1.0,
                redshift: 0.04 * eased,
            }
        }
        VisualPhase::Infalling => {
            let t = normalized_time(elapsed, INFALLING_SECONDS);
            // Radial speed builds through the outer field, then falls toward zero
            // at the apparent horizon to evoke the distant-observer time delay.
            let accelerated = smoothstep(0.0, 1.0, t.powf(1.35));
            let radius = capture_end_radius + (occlusion_radius - capture_end_radius) * accelerated;
            let base_scale = 0.88 + (boundary_base_scale - 0.88) * accelerated;
            // Schwarzschild tidal acceleration grows as the inverse cube of
            // distance. Screen radius is not proper distance, so this monotonic
            // proxy conveys the gradient without claiming a literal scale model.
            let tidal_strength = 0.10 + 0.90 * smoothstep(0.0, 1.0, t.powf(1.45));
            let (radial_scale, tangential_scale) = tidal_scales(base_scale, tidal_strength);
            VisualSample {
                position: direction * radius,
                radial_direction: direction,
                radial_scale,
                tangential_scale,
                alpha: 1.0 + (boundary_alpha - 1.0) * smoothstep(0.55, 1.0, t),
                redshift: 0.04 + (boundary_redshift - 0.04) * smoothstep(0.35, 1.0, t),
            }
        }
        VisualPhase::EnteringEventHorizon => {
            let t = normalized_time(elapsed, EVENT_HORIZON_SECONDS);
            let eased = smoothstep(0.0, 1.0, t);
            let radius = occlusion_radius + (crossing_end_radius - occlusion_radius) * eased;
            let radial_scale =
                boundary_radial_scale + (crossing_radial_scale - boundary_radial_scale) * eased;
            let tangential_scale = boundary_tangential_scale
                + (crossing_tangential_scale - boundary_tangential_scale) * eased;
            VisualSample {
                position: direction * radius,
                radial_direction: direction,
                radial_scale,
                tangential_scale,
                alpha: boundary_alpha * (1.0 - smoothstep(0.82, 1.0, t)),
                redshift: boundary_redshift + (1.0 - boundary_redshift) * eased,
            }
        }
        VisualPhase::Success => VisualSample {
            position: direction * crossing_end_radius,
            radial_direction: direction,
            radial_scale: crossing_radial_scale,
            tangential_scale: crossing_tangential_scale,
            alpha: 0.0,
            redshift: 1.0,
        },
        VisualPhase::Failure => {
            let t = normalized_time(elapsed, FAILURE_SECONDS);
            let direction = if failure.origin.length_squared() > 1.0 {
                failure.origin.normalize()
            } else {
                Vec2::from_angle(fallback_angle)
            };
            let rebound = minimum_extent * 0.24 * ease_out_back(t);
            let recovery = smoothstep(0.0, 0.28, t);
            let pulse = 0.12 * (PI * t).sin();
            let radial_scale =
                failure.radial_scale + (0.62 - failure.radial_scale) * recovery + pulse;
            let tangential_scale =
                failure.tangential_scale + (0.62 - failure.tangential_scale) * recovery + pulse;
            let recovered_alpha = failure.alpha + (1.0 - failure.alpha) * recovery;
            VisualSample {
                position: failure.origin + direction * rebound,
                radial_direction: direction,
                radial_scale,
                tangential_scale,
                alpha: recovered_alpha * (1.0 - smoothstep(0.56, 1.0, t)),
                redshift: 0.0,
            }
        }
    }
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
    let shadow_radius = normalized_drop_target_shadow_radius(lens_radius);
    if screen.length_squared() <= shadow_radius * shadow_radius {
        return true;
    }

    let disk_major = (DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING
        / (CAMERA_DISTANCE_RS * tan_half_fov))
        .min(0.98);
    let disk_minor = (disk_major * (pitch.abs().sin() + 0.10).min(1.0)).max(shadow_radius);
    (screen.x / disk_major).powi(2) + (screen.y / disk_minor).powi(2) <= 1.0
}

fn drag_lens_influence_strength(cursor: Vec2, size: Vec2, lens_radius: f32) -> f32 {
    if !cursor.is_finite() || size.x <= 1.0 || size.y <= 1.0 {
        return 0.0;
    }

    let aspect = size.x / size.y;
    let screen = Vec2::new(
        (cursor.x / size.x * 2.0 - 1.0) * aspect,
        1.0 - cursor.y / size.y * 2.0,
    );
    let tan_half_fov = render_tan_half_fov(lens_radius);
    let shadow_radius = normalized_rendered_shadow_radius(lens_radius);
    let disk_major =
        DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING / (CAMERA_DISTANCE_RS * tan_half_fov);
    let influence_radius = (shadow_radius
        * BACKGROUND_INFLUENCE_SHADOW_RADII
        * clamped_lens_influence_scale(lens_radius))
    .clamp(0.12, 3.0)
    .max(shadow_radius * 1.08)
    .min(disk_major);
    let coordinate = ((screen.length() - shadow_radius)
        / (influence_radius - shadow_radius).max(0.01))
    .clamp(0.0, 1.0);
    1.0 - smoothstep(0.62, 1.0, coordinate)
}

fn normalized_rendered_shadow_radius(lens_radius: f32) -> f32 {
    CRITICAL_IMPACT_PARAMETER_RS / (CAMERA_DISTANCE_RS * render_tan_half_fov(lens_radius))
}

fn normalized_drop_target_shadow_radius(lens_radius: f32) -> f32 {
    normalized_rendered_shadow_radius(lens_radius) * SHADOW_INTERACTION_PADDING
}

fn rendered_shadow_radius(viewport_size: Vec2, lens_radius: f32) -> f32 {
    viewport_size.y.max(1.0) * 0.5 * normalized_rendered_shadow_radius(lens_radius)
}

fn icon_radial_half_extent(kind: DropVisualKind, radial_direction: Vec2) -> f32 {
    let half_size = match kind {
        DropVisualKind::File => Vec2::new(27.0, 30.0),
        DropVisualKind::Application => Vec2::splat(29.0),
    };
    radial_direction.x.abs() * half_size.x + radial_direction.y.abs() * half_size.y
}

fn tidal_scales(base_scale: f32, strength: f32) -> (f32, f32) {
    const LOG_STRAIN: f32 = 0.22;
    let strain = LOG_STRAIN * strength.clamp(0.0, 1.25);
    (
        base_scale * (2.0 * strain).exp(),
        base_scale * (-strain).exp(),
    )
}

fn shadow_visibility(
    parent_position: Vec2,
    radial_direction: Vec2,
    radial_scale: f32,
    child_offset: Vec2,
    child_size: Vec2,
    shadow_radius: f32,
) -> f32 {
    if parent_position.length_squared() <= f32::EPSILON {
        return 0.0;
    }

    let center_radius =
        parent_position.dot(radial_direction) + child_offset.dot(radial_direction) * radial_scale;
    let half_radial_extent = 0.5
        * radial_scale
        * (radial_direction.x.abs() * child_size.x + radial_direction.y.abs() * child_size.y)
            .max(0.001);
    let outside_fraction =
        (center_radius + half_radial_extent - shadow_radius) / (2.0 * half_radial_extent);
    smoothstep(0.0, 1.0, outside_fraction)
}

fn normalized_time(elapsed: f32, duration: f32) -> f32 {
    (elapsed / duration).clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn rotate_2d(value: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(cos * value.x - sin * value.y, sin * value.x + cos * value.y)
}

fn cubic_hermite_progress(value: f32, start_slope: f32, end_slope: f32) -> f32 {
    let t = value.clamp(0.0, 1.0);
    let t2 = t * t;
    let t3 = t2 * t;
    let progress =
        (-2.0 * t3 + 3.0 * t2) + start_slope * (t3 - 2.0 * t2 + t) + end_slope * (t3 - t2);
    progress.clamp(0.0, 1.0)
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

fn apply_gravitational_redshift(rgb: [f32; 3], amount: f32) -> [f32; 3] {
    let amount = amount.clamp(0.0, 1.0);
    let shifted = mix_rgb(rgb, [1.0, 0.055, 0.008], amount * 0.88);
    let remaining_energy = 1.0 - 0.68 * amount;
    [
        shifted[0] * remaining_energy,
        shifted[1] * remaining_energy,
        shifted[2] * remaining_energy,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visual_sample(
        phase: VisualPhase,
        elapsed: f32,
        start_position: Vec2,
        failure: FailureVisualState,
        infall_lane: u8,
        viewport_size: Vec2,
        lens_radius: f32,
    ) -> VisualSample {
        super::visual_sample(
            phase,
            elapsed,
            start_position,
            failure,
            DropVisualKind::File,
            infall_lane,
            viewport_size,
            lens_radius,
        )
    }

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
        assert_eq!(visual.phase, VisualPhase::Infalling);
    }

    #[test]
    fn visual_waits_at_horizon_until_success_is_reported() {
        let mut visual = test_visual(true);

        advance_visual(&mut visual, ATTRACTING_SECONDS);
        advance_visual(&mut visual, CAPTURING_SECONDS);
        let ready = advance_visual(&mut visual, INFALLING_SECONDS);
        assert_eq!(visual.phase, VisualPhase::EnteringEventHorizon);
        assert!(visual.operation_ready_emitted);
        assert_eq!(ready.operation_ready, Some(visual.id));

        let held = advance_visual(&mut visual, EVENT_HORIZON_SECONDS * 2.0);
        assert_eq!(visual.phase, VisualPhase::EnteringEventHorizon);
        assert_eq!(visual.elapsed, 0.0);
        assert_eq!(held.operation_ready, None);

        visual.operation_succeeded = true;
        advance_visual(&mut visual, EVENT_HORIZON_SECONDS);
        assert_eq!(visual.phase, VisualPhase::Success);
    }

    #[test]
    fn infall_moves_radially_inward_without_spin_and_entry_fades() {
        let size = Vec2::new(900.0, 700.0);
        let start = Vec2::new(330.0, 120.0);
        let lens_radius = 1.0;
        let infall_start = visual_sample(
            VisualPhase::Infalling,
            0.0,
            start,
            failure_visual(start),
            0,
            size,
            lens_radius,
        );
        let infall_end = visual_sample(
            VisualPhase::Infalling,
            INFALLING_SECONDS,
            start,
            failure_visual(start),
            0,
            size,
            lens_radius,
        );
        let entering_end = visual_sample(
            VisualPhase::EnteringEventHorizon,
            EVENT_HORIZON_SECONDS,
            start,
            failure_visual(start),
            0,
            size,
            lens_radius,
        );

        assert!(infall_end.position.length() < infall_start.position.length());
        assert!(start.perp_dot(infall_end.position).abs() < 0.01);
        assert!(start.dot(infall_end.position) > 0.0);
        assert!((infall_start.radial_direction - start.normalize()).length() < 0.001);
        assert!((infall_end.radial_direction - start.normalize()).length() < 0.001);
        assert!(infall_end.radial_scale > infall_end.tangential_scale);
        let expected_contact_radius = rendered_shadow_radius(size, lens_radius)
            + icon_radial_half_extent(DropVisualKind::File, start.normalize())
                * infall_end.radial_scale;
        assert!((infall_end.position.length() - expected_contact_radius).abs() < 0.01);
        assert!(entering_end.position.length() < infall_end.position.length());
        assert_eq!(entering_end.alpha, 0.0);
    }

    #[test]
    fn tidal_deformation_preserves_radial_and_tangential_axes_without_rigid_spin() {
        let source = Vec2::new(17.0, -9.0);
        for step in 0..16 {
            let angle = TAU * step as f32 / 16.0;
            let radial = Vec2::from_angle(angle);
            let tangential = radial.perp();

            let deform = |value: Vec2, radial_scale: f32, tangential_scale: f32| {
                let local = rotate_2d(value, -angle);
                rotate_2d(
                    Vec2::new(local.x * radial_scale, local.y * tangential_scale),
                    angle,
                )
            };

            let isotropic = deform(source, 0.73, 0.73);
            assert!((isotropic - source * 0.73).length() < 0.001);

            let stretched_radial = deform(radial, 1.42, 0.61);
            let compressed_tangential = deform(tangential, 1.42, 0.61);
            assert!((stretched_radial - radial * 1.42).length() < 0.001);
            assert!((compressed_tangential - tangential * 0.61).length() < 0.001);
        }
    }

    #[test]
    fn tidal_aspect_ratio_grows_monotonically_during_infall() {
        let size = Vec2::new(900.0, 700.0);
        let start = Vec2::new(320.0, -140.0);
        let mut previous_ratio = 1.0;

        for step in 0..=64 {
            let sample = visual_sample(
                VisualPhase::Infalling,
                INFALLING_SECONDS * step as f32 / 64.0,
                start,
                failure_visual(start),
                0,
                size,
                1.0,
            );
            let ratio = sample.radial_scale / sample.tangential_scale;
            assert!(ratio + f32::EPSILON >= previous_ratio);
            assert!(start.perp_dot(sample.position).abs() < 0.01);
            previous_ratio = ratio;
        }

        assert!(previous_ratio > 1.8);
    }

    #[test]
    fn file_and_application_visuals_touch_the_rendered_shadow_before_crossing() {
        let size = Vec2::new(900.0, 700.0);
        let shadow = rendered_shadow_radius(size, 1.0);

        for kind in [DropVisualKind::File, DropVisualKind::Application] {
            for step in 0..16 {
                let direction = Vec2::from_angle(TAU * step as f32 / 16.0);
                let start = direction * 330.0;
                let sample = super::visual_sample(
                    VisualPhase::Infalling,
                    INFALLING_SECONDS,
                    start,
                    failure_visual(start),
                    kind,
                    0,
                    size,
                    1.0,
                );
                let inner_edge = sample.position.length()
                    - icon_radial_half_extent(kind, direction) * sample.radial_scale;
                assert!((inner_edge - shadow).abs() < 0.01);
            }
        }
    }

    #[test]
    fn all_capture_phases_start_at_release_and_move_only_radially_inward() {
        let size = Vec2::new(900.0, 700.0);
        let lens_radius = 1.0;
        let inside_shadow = Vec2::new(rendered_shadow_radius(size, lens_radius) * 0.35, 0.0);

        for lane in 0..4 {
            for start in [
                Vec2::new(330.0, 120.0),
                Vec2::new(-260.0, 150.0),
                inside_shadow,
                Vec2::ZERO,
            ] {
                let expected_direction = if start.length_squared() > f32::EPSILON {
                    start.normalize()
                } else {
                    Vec2::from_angle((lane as f32) * TAU / 4.0 + PI * 0.125)
                };
                let first = visual_sample(
                    VisualPhase::Attracting,
                    0.0,
                    start,
                    failure_visual(start),
                    lane,
                    size,
                    lens_radius,
                );
                assert!((first.position - start).length() < 0.001);
                let mut previous_radius = start.length();
                for (phase, duration) in [
                    (VisualPhase::Attracting, ATTRACTING_SECONDS),
                    (VisualPhase::Capturing, CAPTURING_SECONDS),
                    (VisualPhase::Infalling, INFALLING_SECONDS),
                    (VisualPhase::EnteringEventHorizon, EVENT_HORIZON_SECONDS),
                ] {
                    for step in 0..=32 {
                        let sample = visual_sample(
                            phase,
                            duration * step as f32 / 32.0,
                            start,
                            failure_visual(start),
                            lane,
                            size,
                            lens_radius,
                        );
                        let radius = sample.position.length();
                        assert!(
                            radius <= previous_radius + 0.001,
                            "lane {lane}, phase {phase:?}, step {step}: {radius} > {previous_radius}"
                        );
                        assert!((sample.radial_direction - expected_direction).length() < 0.001);
                        assert!(
                            expected_direction.perp_dot(sample.position).abs() < 0.01,
                            "lane {lane}, phase {phase:?}, step {step}: path left its radial line"
                        );
                        if radius > f32::EPSILON {
                            assert!(expected_direction.dot(sample.position) > 0.0);
                        }
                        previous_radius = radius;
                    }
                }
            }
        }
    }

    #[test]
    fn attraction_and_capture_match_position_and_scale_velocity() {
        let size = Vec2::new(900.0, 700.0);
        let lens_radius = 1.0;
        let shadow_radius = rendered_shadow_radius(size, lens_radius);
        let delta = 0.000_5;

        for start in [
            Vec2::new(330.0, 120.0),
            Vec2::new(shadow_radius + ICON_MAX_HALF_DIAGONAL + 10.0, 0.0),
            Vec2::new(shadow_radius * 0.8, 0.0),
            Vec2::new(3.0, 0.0),
            Vec2::ZERO,
        ] {
            let attraction_before = visual_sample(
                VisualPhase::Attracting,
                ATTRACTING_SECONDS - delta,
                start,
                failure_visual(start),
                0,
                size,
                lens_radius,
            );
            let attraction_end = visual_sample(
                VisualPhase::Attracting,
                ATTRACTING_SECONDS,
                start,
                failure_visual(start),
                0,
                size,
                lens_radius,
            );
            let capture_start = visual_sample(
                VisualPhase::Capturing,
                0.0,
                start,
                failure_visual(start),
                0,
                size,
                lens_radius,
            );
            let capture_after = visual_sample(
                VisualPhase::Capturing,
                delta,
                start,
                failure_visual(start),
                0,
                size,
                lens_radius,
            );

            assert!((attraction_end.position - capture_start.position).length() < 0.001);
            assert!(
                (attraction_end.radial_scale - capture_start.radial_scale).abs() < f32::EPSILON
            );
            assert!(
                (attraction_end.tangential_scale - capture_start.tangential_scale).abs()
                    < f32::EPSILON
            );
            let attraction_position_rate =
                (attraction_end.position.length() - attraction_before.position.length()) / delta;
            let capture_position_rate =
                (capture_after.position.length() - capture_start.position.length()) / delta;
            let attraction_scale_rate =
                (attraction_end.radial_scale - attraction_before.radial_scale) / delta;
            let capture_scale_rate =
                (capture_after.radial_scale - capture_start.radial_scale) / delta;
            assert!((attraction_position_rate - capture_position_rate).abs() < 2.0);
            assert!((attraction_scale_rate - capture_scale_rate).abs() < 0.02);
        }
    }

    #[test]
    fn every_capture_boundary_is_continuous_in_position_shape_and_redshift() {
        let size = Vec2::new(900.0, 700.0);
        let start = Vec2::new(-280.0, 145.0);
        let failure = failure_visual(start);
        let at = |phase, elapsed| visual_sample(phase, elapsed, start, failure, 0, size, 1.0);

        for (left, right) in [
            (
                at(VisualPhase::Attracting, ATTRACTING_SECONDS),
                at(VisualPhase::Capturing, 0.0),
            ),
            (
                at(VisualPhase::Capturing, CAPTURING_SECONDS),
                at(VisualPhase::Infalling, 0.0),
            ),
            (
                at(VisualPhase::Infalling, INFALLING_SECONDS),
                at(VisualPhase::EnteringEventHorizon, 0.0),
            ),
        ] {
            assert!((left.position - right.position).length() < 0.001);
            assert!((left.radial_scale - right.radial_scale).abs() < 0.000_01);
            assert!((left.tangential_scale - right.tangential_scale).abs() < 0.000_01);
            assert!((left.alpha - right.alpha).abs() < 0.000_01);
            assert!((left.redshift - right.redshift).abs() < 0.000_01);
        }
    }

    #[test]
    fn icon_inner_edge_stays_outside_the_shadow_before_success() {
        let size = Vec2::new(900.0, 700.0);
        let lens_radius = 1.0;
        let shadow_radius = rendered_shadow_radius(size, lens_radius);

        for lane in 0..4 {
            for start in [
                Vec2::new(330.0, 120.0),
                Vec2::new(shadow_radius + ICON_MAX_HALF_DIAGONAL + 1.0, 0.0),
                Vec2::new(-(shadow_radius + ICON_MAX_HALF_DIAGONAL + 24.0), 18.0),
            ] {
                for (phase, duration) in [
                    (VisualPhase::Attracting, ATTRACTING_SECONDS),
                    (VisualPhase::Capturing, CAPTURING_SECONDS),
                    (VisualPhase::Infalling, INFALLING_SECONDS),
                ] {
                    for step in 0..=64 {
                        let sample = visual_sample(
                            phase,
                            duration * step as f32 / 64.0,
                            start,
                            failure_visual(start),
                            lane,
                            size,
                            lens_radius,
                        );
                        let inner_edge = sample.position.length()
                            - icon_radial_half_extent(
                                DropVisualKind::File,
                                sample.radial_direction,
                            ) * sample.radial_scale;
                        assert!(
                            inner_edge + 0.001 >= shadow_radius,
                            "lane {lane}, phase {phase:?}, step {step}: {inner_edge} < {shadow_radius}"
                        );
                    }
                }

                let waiting = visual_sample(
                    VisualPhase::EnteringEventHorizon,
                    0.0,
                    start,
                    failure_visual(start),
                    lane,
                    size,
                    lens_radius,
                );
                let waiting_inner_edge = waiting.position.length()
                    - icon_radial_half_extent(DropVisualKind::File, waiting.radial_direction)
                        * waiting.radial_scale;
                assert!(waiting_inner_edge + 0.001 >= shadow_radius);
            }
        }
    }

    #[test]
    fn horizon_crossing_occludes_once_and_never_reappears() {
        let size = Vec2::new(900.0, 700.0);
        let start = Vec2::new(280.0, -130.0);
        let lens_radius = 1.0;
        let boundary = visual_sample(
            VisualPhase::EnteringEventHorizon,
            0.0,
            start,
            failure_visual(start),
            0,
            size,
            lens_radius,
        );
        let mut previous_alpha = boundary.alpha;

        assert!(boundary.alpha > 0.0);
        for step in 1..=32 {
            let sample = visual_sample(
                VisualPhase::EnteringEventHorizon,
                EVENT_HORIZON_SECONDS * step as f32 / 32.0,
                start,
                failure_visual(start),
                0,
                size,
                lens_radius,
            );
            assert!(sample.alpha <= previous_alpha + f32::EPSILON);
            previous_alpha = sample.alpha;
        }

        let success = visual_sample(
            VisualPhase::Success,
            SUCCESS_SECONDS,
            start,
            failure_visual(start),
            0,
            size,
            lens_radius,
        );
        assert_eq!(previous_alpha, 0.0);
        assert_eq!(success.alpha, 0.0);
    }

    #[test]
    fn horizon_crossing_swallows_core_tiles_from_the_inner_edge_without_reappearing() {
        let size = Vec2::new(900.0, 700.0);
        let start = Vec2::new(290.0, 170.0);
        let shadow = rendered_shadow_radius(size, 1.0);
        let cell_size = Vec2::new(38.0, 48.0) / 4.0;
        let lower_left = -Vec2::new(38.0, 48.0) * 0.5 + cell_size * 0.5;
        let offsets = (0..4)
            .flat_map(|y| {
                (0..4).map(move |x| lower_left + Vec2::new(x as f32, y as f32) * cell_size)
            })
            .collect::<Vec<_>>();
        let mut previous = vec![1.0; offsets.len()];

        for step in 0..=64 {
            let sample = visual_sample(
                VisualPhase::EnteringEventHorizon,
                EVENT_HORIZON_SECONDS * step as f32 / 64.0,
                start,
                failure_visual(start),
                0,
                size,
                1.0,
            );
            for (index, offset) in offsets.iter().copied().enumerate() {
                let visibility = sample.alpha
                    * shadow_visibility(
                        sample.position,
                        sample.radial_direction,
                        sample.radial_scale,
                        offset,
                        cell_size,
                        shadow,
                    );
                assert!(
                    visibility <= previous[index] + 0.000_1,
                    "tile {index} reappeared at step {step}: {visibility} > {}",
                    previous[index]
                );
                previous[index] = visibility;
            }
        }

        assert!(previous.into_iter().all(|visibility| visibility == 0.0));
    }

    #[test]
    fn a_release_inside_the_shadow_waits_for_success_before_crossing() {
        let size = Vec2::new(900.0, 700.0);
        let lens_radius = 1.0;
        let start = Vec2::new(rendered_shadow_radius(size, lens_radius) * 0.35, 0.0);
        let mut visual = test_visual(true);
        visual.start_position = start;
        visual.failure = failure_visual(start);

        advance_visual(&mut visual, ATTRACTING_SECONDS);
        advance_visual(&mut visual, CAPTURING_SECONDS);
        let ready = advance_visual(&mut visual, INFALLING_SECONDS);
        assert_eq!(ready.operation_ready, Some(visual.id));
        assert_eq!(visual.phase, VisualPhase::EnteringEventHorizon);

        let boundary = visual_sample(
            visual.phase,
            visual.elapsed,
            visual.start_position,
            visual.failure,
            visual.infall_lane,
            size,
            lens_radius,
        );
        advance_visual(&mut visual, EVENT_HORIZON_SECONDS * 2.0);
        let held = visual_sample(
            visual.phase,
            visual.elapsed,
            visual.start_position,
            visual.failure,
            visual.infall_lane,
            size,
            lens_radius,
        );

        assert_eq!(visual.phase, VisualPhase::EnteringEventHorizon);
        assert_eq!(visual.elapsed, 0.0);
        assert!((held.position - boundary.position).length() < 0.001);
        assert_eq!(held.alpha, boundary.alpha);

        visual.operation_succeeded = true;
        advance_visual(&mut visual, EVENT_HORIZON_SECONDS);
        let success = visual_sample(
            visual.phase,
            visual.elapsed,
            visual.start_position,
            visual.failure,
            visual.infall_lane,
            size,
            lens_radius,
        );
        assert_eq!(visual.phase, VisualPhase::Success);
        assert!(success.position.length() <= boundary.position.length());
        assert_eq!(success.alpha, 0.0);
    }

    #[test]
    fn success_is_accepted_only_after_the_visual_reaches_the_boundary() {
        let mut visual = test_visual(true);

        assert!(!can_accept_operation_success(&visual));
        visual.phase = VisualPhase::EnteringEventHorizon;
        assert!(!can_accept_operation_success(&visual));
        visual.operation_ready_emitted = true;
        assert!(can_accept_operation_success(&visual));
        visual.phase = VisualPhase::Attracting;
        assert!(!can_accept_operation_success(&visual));
    }

    #[test]
    fn releases_on_either_side_of_the_shadow_have_continuous_infall() {
        let size = Vec2::new(900.0, 700.0);
        let lens_radius = 1.0;
        let shadow = rendered_shadow_radius(size, lens_radius);
        let sample_at = |radius: f32| {
            let start = Vec2::new(radius, 0.0);
            visual_sample(
                VisualPhase::Infalling,
                INFALLING_SECONDS,
                start,
                failure_visual(start),
                0,
                size,
                lens_radius,
            )
        };

        let just_inside = sample_at(shadow - 0.5);
        let just_outside = sample_at(shadow + 0.5);

        assert!((just_inside.position.length() - just_outside.position.length()).abs() < 2.0);
        assert!((just_inside.radial_scale - just_outside.radial_scale).abs() < 0.01);
        assert!((just_inside.tangential_scale - just_outside.tangential_scale).abs() < 0.01);
        assert!((just_inside.alpha - just_outside.alpha).abs() < 0.02);
    }

    #[test]
    fn horizon_occlusion_progresses_across_each_icon_part() {
        let shadow_radius = 80.0;
        let scale = 0.4;
        let size = Vec2::splat(50.0);

        let outside = shadow_visibility(
            Vec2::new(90.0, 0.0),
            Vec2::X,
            scale,
            Vec2::ZERO,
            size,
            shadow_radius,
        );
        let straddling = shadow_visibility(
            Vec2::new(80.0, 0.0),
            Vec2::X,
            scale,
            Vec2::ZERO,
            size,
            shadow_radius,
        );
        let inside = shadow_visibility(
            Vec2::new(70.0, 0.0),
            Vec2::X,
            scale,
            Vec2::ZERO,
            size,
            shadow_radius,
        );

        assert_eq!(outside, 1.0);
        assert!((straddling - 0.5).abs() < f32::EPSILON);
        assert_eq!(inside, 0.0);
    }

    #[test]
    fn failure_feedback_starts_from_the_last_visible_scale_and_alpha() {
        let failure = FailureVisualState {
            origin: Vec2::new(90.0, 20.0),
            radial_scale: 0.83,
            tangential_scale: 0.46,
            alpha: 0.47,
        };
        let sample = visual_sample(
            VisualPhase::Failure,
            0.0,
            failure.origin,
            failure,
            0,
            Vec2::new(900.0, 700.0),
            1.0,
        );

        assert!((sample.radial_scale - failure.radial_scale).abs() < f32::EPSILON);
        assert!((sample.tangential_scale - failure.tangential_scale).abs() < f32::EPSILON);
        assert!((sample.alpha - failure.alpha).abs() < f32::EPSILON);
        assert_eq!(sample.position, failure.origin);
    }

    #[test]
    fn phase_overshoot_is_carried_into_the_next_motion_segment() {
        let mut visual = test_visual(true);
        visual.elapsed = ATTRACTING_SECONDS - 0.02;

        advance_visual(&mut visual, 0.10);

        assert_eq!(visual.phase, VisualPhase::Capturing);
        assert!((visual.elapsed - 0.08).abs() < 0.000_01);
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

    #[test]
    fn drag_lens_influence_covers_center_acceptance_and_influence_boundaries() {
        let size = Vec2::new(900.0, 700.0);
        let lens_radius = 1.0;
        let center = size * 0.5;
        let shadow_radius = normalized_rendered_shadow_radius(lens_radius);
        let acceptance_radius = normalized_drop_target_shadow_radius(lens_radius);
        let influence_radius = test_drag_influence_radius(lens_radius);

        assert_eq!(drag_lens_influence_strength(center, size, lens_radius), 1.0);
        assert_eq!(
            drag_lens_influence_strength(
                cursor_at_screen_radius(size, shadow_radius),
                size,
                lens_radius,
            ),
            1.0,
        );
        assert_eq!(
            drag_lens_influence_strength(
                cursor_at_screen_radius(size, acceptance_radius),
                size,
                lens_radius,
            ),
            1.0,
        );

        let just_inside = drag_lens_influence_strength(
            cursor_at_screen_radius(size, influence_radius - 0.001),
            size,
            lens_radius,
        );
        let at_boundary = drag_lens_influence_strength(
            cursor_at_screen_radius(size, influence_radius),
            size,
            lens_radius,
        );
        let outside = drag_lens_influence_strength(
            cursor_at_screen_radius(size, influence_radius + 0.001),
            size,
            lens_radius,
        );

        assert!(just_inside > 0.0);
        assert!(at_boundary <= 0.000_01);
        assert_eq!(outside, 0.0);
    }

    #[test]
    fn maximum_drag_influence_does_not_cross_visible_disk_edge() {
        let lens_radius = crate::settings::LENS_INFLUENCE_SCALE_MAX;
        let tan_half_fov = render_tan_half_fov(lens_radius);
        let disk_major =
            DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING / (CAMERA_DISTANCE_RS * tan_half_fov);
        let influence_radius = test_drag_influence_radius(lens_radius);

        assert!(influence_radius <= disk_major);
        assert!(influence_radius / disk_major > 0.998);
    }

    #[test]
    fn drag_lens_influence_rejects_invalid_cursor_and_degenerate_windows() {
        let center = Vec2::splat(0.5);
        for size in [
            Vec2::ZERO,
            Vec2::new(0.5, 700.0),
            Vec2::new(900.0, 0.5),
            Vec2::ONE,
        ] {
            assert_eq!(drag_lens_influence_strength(center, size, 1.0), 0.0);
        }

        let barely_valid = Vec2::splat(1.000_1);
        assert_eq!(
            drag_lens_influence_strength(barely_valid * 0.5, barely_valid, 1.0),
            1.0,
        );

        let size = Vec2::new(900.0, 700.0);
        for cursor in [
            Vec2::new(f32::NAN, 0.0),
            Vec2::new(f32::INFINITY, 0.0),
            Vec2::new(0.0, f32::NEG_INFINITY),
        ] {
            assert_eq!(drag_lens_influence_strength(cursor, size, 1.0), 0.0);
        }
    }

    #[test]
    fn drag_lens_influence_stays_finite_for_extreme_and_non_finite_lens_values() {
        let size = Vec2::new(900.0, 700.0);
        for lens_radius in [
            -1.0e6,
            0.0,
            0.4,
            2.0,
            1.0e6,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::NAN,
        ] {
            let shadow_radius = normalized_rendered_shadow_radius(lens_radius);
            let samples = [
                size * 0.5,
                cursor_at_screen_radius(size, shadow_radius),
                Vec2::new(-size.x, -size.y),
            ];
            for cursor in samples {
                let strength = drag_lens_influence_strength(cursor, size, lens_radius);
                assert!(
                    strength.is_finite() && (0.0..=1.0).contains(&strength),
                    "lens {lens_radius:?}, cursor {cursor:?}: {strength}"
                );
            }
            assert_eq!(
                drag_lens_influence_strength(size * 0.5, size, lens_radius),
                1.0,
            );
        }
    }

    fn cursor_at_screen_radius(size: Vec2, screen_radius: f32) -> Vec2 {
        Vec2::new(size.x * 0.5 + screen_radius * size.y * 0.5, size.y * 0.5)
    }

    fn test_drag_influence_radius(lens_radius: f32) -> f32 {
        let tan_half_fov = render_tan_half_fov(lens_radius);
        let shadow_radius = normalized_rendered_shadow_radius(lens_radius);
        let disk_major =
            DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING / (CAMERA_DISTANCE_RS * tan_half_fov);
        (shadow_radius
            * BACKGROUND_INFLUENCE_SHADOW_RADII
            * clamped_lens_influence_scale(lens_radius))
        .clamp(0.12, 3.0)
        .max(shadow_radius * 1.08)
        .min(disk_major)
    }

    fn test_visual(authorized: bool) -> DropVisual {
        DropVisual {
            id: 7,
            kind: DropVisualKind::File,
            phase: VisualPhase::Attracting,
            elapsed: 0.0,
            start_position: Vec2::new(200.0, 100.0),
            failure: failure_visual(Vec2::new(200.0, 100.0)),
            infall_lane: 0,
            authorized,
            operation_ready_emitted: false,
            operation_succeeded: false,
        }
    }

    fn failure_visual(origin: Vec2) -> FailureVisualState {
        FailureVisualState {
            origin,
            radial_scale: 1.0,
            tangential_scale: 1.0,
            alpha: 1.0,
        }
    }
}
