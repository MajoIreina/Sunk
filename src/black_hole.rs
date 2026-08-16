use std::path::PathBuf;

use bevy::{
    app::AppExit,
    asset::{load_internal_asset, uuid_handle},
    camera::ClearColorConfig,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    mesh::MeshVertexBufferLayoutRef,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState,
        RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    render::view::screenshot::{Screenshot, save_to_disk},
    shader::{Shader, ShaderRef},
    sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin},
    window::{CursorOptions, PrimaryWindow, WindowResized},
};

use crate::desktop_capture::{DesktopCaptureState, DesktopCaptureSystems};
use crate::file_interaction::{DropInteractionState, FileInteractionSystems};
use crate::physics::{EVENT_HORIZON_RS, ISCO_RS};
use crate::settings::{BlackHoleSettings, clamped_lens_influence_scale};
use crate::window_interaction::render_tan_half_fov;

const BLACK_HOLE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("4b4ec109-cf32-41d7-9fc0-4217ab854018");
const INITIAL_WIDTH: f32 = 900.0;
const INITIAL_HEIGHT: f32 = 700.0;
const CAMERA_DISTANCE_RS: f32 = 30.0;
const DISK_OUTER_RADIUS_RS: f32 = 11.5;
const DISK_LENSING_PADDING: f32 = 1.10;
const CRITICAL_IMPACT_PARAMETER_RS: f32 = 2.598_076;
const BACKGROUND_INFLUENCE_SHADOW_RADII: f32 = 3.45;
const SSAA_SAMPLE_OFFSETS: [Vec2; 4] = [
    Vec2::new(-0.25, -0.25),
    Vec2::new(0.25, -0.25),
    Vec2::new(-0.25, 0.25),
    Vec2::new(0.25, 0.25),
];

pub struct BlackHolePlugin;

impl Plugin for BlackHolePlugin {
    fn build(&self, app: &mut App) {
        // `include_str!` inside Bevy's internal-asset macro makes the WGSL part of
        // the executable. Release builds no longer depend on an adjacent `assets`
        // directory, while the standalone source file remains editable in-tree.
        load_internal_asset!(
            app,
            BLACK_HOLE_SHADER_HANDLE,
            "../assets/shaders/black_hole.wgsl",
            Shader::from_wgsl
        );

        app.add_plugins(Material2dPlugin::<BlackHoleMaterial>::default())
            .init_resource::<BlackHoleControls>()
            .init_resource::<BlackHoleSettings>()
            .init_resource::<QaFrameCapture>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    resize_canvas,
                    handle_controls,
                    sync_material
                        .after(handle_controls)
                        .after(FileInteractionSystems::ObserveDrops)
                        .after(DesktopCaptureSystems::Sync),
                    capture_qa_frame.after(sync_material),
                ),
            );
    }
}

#[derive(Resource)]
struct QaFrameCapture {
    path: Option<PathBuf>,
    min_desktop_frame: u64,
    requested: bool,
}

impl Default for QaFrameCapture {
    fn default() -> Self {
        Self {
            path: std::env::var_os("SUNK_CAPTURE_FRAME").map(PathBuf::from),
            min_desktop_frame: std::env::var("SUNK_CAPTURE_AFTER_DESKTOP_FRAME")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(10),
            requested: false,
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct BlackHoleMaterial {
    #[uniform(0)]
    params: BlackHoleUniform,
    #[texture(1)]
    #[sampler(2)]
    desktop_texture: Option<Handle<Image>>,
}

impl Material2d for BlackHoleMaterial {
    fn fragment_shader() -> ShaderRef {
        BLACK_HOLE_SHADER_HANDLE.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let additive = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        if let Some(target) = descriptor
            .fragment
            .as_mut()
            .and_then(|fragment| fragment.targets.first_mut())
            .and_then(Option::as_mut)
        {
            // Each pass returns weighted premultiplied RGBA. Adding all visible
            // passes is therefore the exact spatial average for both radiance and
            // optical coverage, without applying source alpha a second time.
            target.blend = Some(BlendState {
                color: additive,
                alpha: additive,
            });
        }
        Ok(())
    }
}

#[derive(ShaderType, Debug, Clone, Copy)]
struct BlackHoleUniform {
    /// width, height, simulation time, tan(vertical fov / 2)
    viewport_time_fov: Vec4,
    /// yaw, pitch, camera distance (in Schwarzschild radii), roll
    camera: Vec4,
    /// inner radius, outer radius, half thickness, angular speed
    disk: Vec4,
    /// horizon radius, maximum steps, integrator selector, turn tolerance
    integration: Vec4,
    /// disk optical depth, emission gain, exposure, horizon opacity
    appearance: Vec4,
    /// peak temperature K, cloud optical depth, turbulence, cloudiness
    material: Vec4,
    /// linear RGB disk tint, reserved
    tint: Vec4,
    /// desktop enabled, replacement strength, lens radius, reserved
    desktop: Vec4,
    /// captured texture origin.xy and scale.xy
    desktop_uv_origin_scale: Vec4,
    /// subpixel offset.xy (in pixels), sample weight, reserved
    sample: Vec4,
    /// native drag cursor UV.xy, lens influence, capture-target state
    drag_feedback: Vec4,
}

impl Default for BlackHoleUniform {
    fn default() -> Self {
        Self {
            viewport_time_fov: Vec4::new(
                INITIAL_WIDTH,
                INITIAL_HEIGHT,
                0.0,
                (42.0_f32.to_radians() * 0.5).tan(),
            ),
            camera: Vec4::new(-0.34, 0.26, CAMERA_DISTANCE_RS, 0.0),
            disk: Vec4::new(ISCO_RS, DISK_OUTER_RADIUS_RS, 0.14, 0.34),
            integration: Vec4::new(EVENT_HORIZON_RS, 192.0, 1.0, 0.050),
            appearance: Vec4::new(7.0, 3.6, 1.0, 1.0),
            material: Vec4::new(10_500.0, 0.55, 0.72, 0.78),
            tint: Vec4::new(1.0, 0.72, 0.48, 1.0),
            desktop: Vec4::new(0.0, 1.0, 0.78, 0.0),
            desktop_uv_origin_scale: Vec4::new(0.0, 0.0, 1.0, 1.0),
            sample: Vec4::new(0.0, 0.0, 1.0, 0.0),
            drag_feedback: Vec4::ZERO,
        }
    }
}

#[derive(Resource)]
struct BlackHoleMaterialHandles(Vec<Handle<BlackHoleMaterial>>);

#[derive(Component)]
struct BlackHoleCanvas {
    sample_index: usize,
}

#[derive(Resource, Debug)]
pub(crate) struct BlackHoleControls {
    yaw: f32,
    pitch: f32,
    elapsed: f32,
    paused: bool,
    pass_through: bool,
}

impl Default for BlackHoleControls {
    fn default() -> Self {
        Self {
            yaw: -0.34,
            pitch: 0.26,
            elapsed: 0.0,
            paused: false,
            pass_through: false,
        }
    }
}

impl BlackHoleControls {
    pub(crate) fn pitch(&self) -> f32 {
        self.pitch
    }

    pub(crate) fn pass_through(&self) -> bool {
        self.pass_through
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BlackHoleMaterial>>,
    desktop_capture: Res<DesktopCaptureState>,
) {
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        Msaa::Off,
    ));

    let mesh = meshes.add(Rectangle::default());
    let mut material_handles = Vec::with_capacity(SSAA_SAMPLE_OFFSETS.len());
    for sample_index in 0..SSAA_SAMPLE_OFFSETS.len() {
        let mut params = BlackHoleUniform {
            desktop_uv_origin_scale: desktop_capture.uv_origin_scale,
            ..default()
        };
        params.sample = sample_parameters(sample_index, 1);
        let material = materials.add(BlackHoleMaterial {
            params,
            desktop_texture: desktop_capture.texture.clone(),
        });

        commands.spawn((
            Mesh2d(mesh.clone()),
            MeshMaterial2d(material.clone()),
            Transform::from_scale(Vec3::new(INITIAL_WIDTH, INITIAL_HEIGHT, 1.0)),
            if sample_index == 0 {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
            BlackHoleCanvas { sample_index },
        ));
        material_handles.push(material);
    }
    commands.insert_resource(BlackHoleMaterialHandles(material_handles));

    info!(
        "Sunk controls: left drag=move window, right drag=orbit, wheel=zoom, \
         Space=pause, R=reset, P=mouse passthrough, F1=settings, Esc=quit"
    );
}

fn resize_canvas(
    mut resized: MessageReader<WindowResized>,
    primary_window: Single<Entity, With<PrimaryWindow>>,
    mut canvases: Query<&mut Transform, With<BlackHoleCanvas>>,
) {
    for event in resized.read() {
        if event.window == *primary_window {
            for mut canvas in &mut canvases {
                canvas.scale = Vec3::new(event.width.max(1.0), event.height.max(1.0), 1.0);
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy injects each ECS resource/query as an explicit system parameter"
)]
fn handle_controls(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    mouse_scroll: Res<AccumulatedMouseScroll>,
    mut controls: ResMut<BlackHoleControls>,
    mut settings: ResMut<BlackHoleSettings>,
    mut primary_window: Single<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut app_exit: MessageWriter<AppExit>,
) {
    let primary_focused = primary_window.0.focused;

    if primary_focused && keyboard.just_pressed(KeyCode::Escape) {
        app_exit.write(AppExit::Success);
    }

    if primary_focused && keyboard.just_pressed(KeyCode::Space) {
        controls.paused = !controls.paused;
    }

    if primary_focused && keyboard.just_pressed(KeyCode::KeyR) {
        *controls = BlackHoleControls::default();
        *settings = BlackHoleSettings::default();
        primary_window.1.hit_test = true;
    }

    if primary_focused && keyboard.just_pressed(KeyCode::KeyP) {
        controls.pass_through = !controls.pass_through;
        primary_window.1.hit_test = !controls.pass_through;
        info!("mouse passthrough: {}", controls.pass_through);
    }

    if primary_focused && mouse_buttons.just_pressed(MouseButton::Left) && !controls.pass_through {
        primary_window.0.start_drag_move();
    }

    if primary_focused && mouse_buttons.pressed(MouseButton::Right) {
        controls.yaw -= mouse_motion.delta.x * 0.005;
        controls.pitch = (controls.pitch + mouse_motion.delta.y * 0.005).clamp(-1.25, 1.25);
    }

    if primary_focused && mouse_scroll.delta.y != 0.0 {
        settings.apparent_size =
            (settings.apparent_size * (mouse_scroll.delta.y * 0.08).exp()).clamp(0.25, 3.0);
    }

    if !controls.paused {
        controls.elapsed += time.delta_secs();
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "material synchronization reads independent render, window, and interaction resources"
)]
fn sync_material(
    windows: Single<&Window, With<PrimaryWindow>>,
    controls: Res<BlackHoleControls>,
    settings: Res<BlackHoleSettings>,
    drop_interaction: Res<DropInteractionState>,
    desktop_capture: Res<DesktopCaptureState>,
    material_handles: Res<BlackHoleMaterialHandles>,
    mut materials: ResMut<Assets<BlackHoleMaterial>>,
    mut canvases: Query<(&BlackHoleCanvas, &mut Visibility)>,
) {
    let mut settings = settings.clone();
    settings.sanitize();
    let physical_size = windows.resolution.physical_size().as_vec2().max(Vec2::ONE);
    let spatial_samples = settings.anti_aliasing.spatial_samples();
    let desktop_ready = desktop_capture.is_ready();
    let tan_half_fov = render_tan_half_fov(settings.lens_radius);
    let mut params = BlackHoleUniform {
        viewport_time_fov: Vec4::new(
            physical_size.x,
            physical_size.y,
            controls.elapsed,
            tan_half_fov,
        ),
        camera: Vec4::new(controls.yaw, controls.pitch, CAMERA_DISTANCE_RS, 0.0),
        disk: Vec4::new(
            ISCO_RS,
            DISK_OUTER_RADIUS_RS,
            0.14 * settings.disk_thickness,
            settings.animation_speed,
        ),
        integration: Vec4::new(
            EVENT_HORIZON_RS,
            settings.render_quality.integration_steps() as f32,
            settings.render_quality.integration_quality(),
            settings.render_quality.turn_tolerance(),
        ),
        appearance: Vec4::new(
            settings.disk_density,
            settings.emission_strength,
            settings.exposure,
            1.0,
        ),
        material: Vec4::new(
            settings.disk_temperature_kelvin,
            settings.corona_opacity,
            settings.turbulence,
            settings.cloudiness,
        ),
        tint: Vec4::new(
            settings.disk_tint_linear[0],
            settings.disk_tint_linear[1],
            settings.disk_tint_linear[2],
            1.0,
        ),
        desktop: Vec4::new(
            f32::from(desktop_ready),
            settings.background_warp,
            background_influence_radius(tan_half_fov, CAMERA_DISTANCE_RS, settings.lens_radius),
            0.0,
        ),
        desktop_uv_origin_scale: desktop_capture.uv_origin_scale,
        sample: Vec4::ZERO,
        drag_feedback: Vec4::new(
            drop_interaction.feedback_uv.x,
            drop_interaction.feedback_uv.y,
            if drop_interaction.drag_active {
                drop_interaction.influence_strength
            } else {
                0.0
            },
            f32::from(drop_interaction.hovering_black_hole),
        ),
    };

    for (sample_index, handle) in material_handles.0.iter().enumerate() {
        let Some(mut material) = materials.get_mut(handle) else {
            continue;
        };
        params.sample = sample_parameters(sample_index, spatial_samples);
        material.params = params;
        if material.desktop_texture != desktop_capture.texture {
            material.desktop_texture = desktop_capture.texture.clone();
        }
    }

    for (canvas, mut visibility) in &mut canvases {
        let desired = if canvas.sample_index < spatial_samples as usize {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != desired {
            *visibility = desired;
        }
    }
}

fn sample_parameters(sample_index: usize, spatial_samples: u32) -> Vec4 {
    if spatial_samples >= SSAA_SAMPLE_OFFSETS.len() as u32 {
        let offset = SSAA_SAMPLE_OFFSETS[sample_index];
        Vec4::new(offset.x, offset.y, 0.25, 0.0)
    } else {
        Vec4::new(0.0, 0.0, f32::from(sample_index == 0), 0.0)
    }
}

fn background_influence_radius(tan_half_fov: f32, camera_distance: f32, scale: f32) -> f32 {
    let camera_distance = camera_distance.max(1.01);
    let tan_half_fov = tan_half_fov.max(1.0e-4);
    let shadow_radius = CRITICAL_IMPACT_PARAMETER_RS / (camera_distance * tan_half_fov);
    let requested =
        shadow_radius * BACKGROUND_INFLUENCE_SHADOW_RADII * clamped_lens_influence_scale(scale);
    let disk_major = DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING / (camera_distance * tan_half_fov);
    requested.clamp(0.12, 3.0).min(disk_major)
}

fn capture_qa_frame(
    mut commands: Commands,
    desktop_capture: Res<DesktopCaptureState>,
    mut capture: ResMut<QaFrameCapture>,
) {
    if capture.requested || desktop_capture.frame_index < capture.min_desktop_frame {
        return;
    }
    let Some(path) = capture.path.clone() else {
        return;
    };

    capture.requested = true;
    info!("saving internal QA frame to {}", path.display());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_influence_tracks_apparent_black_hole_size() {
        let base_tan = (42.0_f32.to_radians() * 0.5).tan();
        let small = background_influence_radius(base_tan / 0.5, CAMERA_DISTANCE_RS, 1.0);
        let normal = background_influence_radius(base_tan, CAMERA_DISTANCE_RS, 1.0);
        let large = background_influence_radius(base_tan / 2.0, CAMERA_DISTANCE_RS, 1.0);

        assert!(small < normal && normal < large);
        assert!((large / normal - 2.0).abs() < 1.0e-5);
    }

    #[test]
    fn maximum_background_influence_stops_at_visible_disk_edge() {
        let base_tan = (42.0_f32.to_radians() * 0.5).tan();
        for tan_half_fov in [base_tan * 0.5, base_tan, base_tan * 2.0] {
            let influence = background_influence_radius(
                tan_half_fov,
                CAMERA_DISTANCE_RS,
                crate::settings::LENS_INFLUENCE_SCALE_MAX,
            );
            let disk_major =
                DISK_OUTER_RADIUS_RS * DISK_LENSING_PADDING / (CAMERA_DISTANCE_RS * tan_half_fov);
            assert!(influence <= disk_major);
            assert!(influence / disk_major > 0.998);
        }
    }

    #[test]
    fn ssaa_passes_form_a_centered_unit_weight_grid() {
        let samples = (0..4)
            .map(|index| sample_parameters(index, 4))
            .collect::<Vec<_>>();
        let offset_sum = samples.iter().map(|sample| sample.xy()).sum::<Vec2>();
        let weight_sum = samples.iter().map(|sample| sample.z).sum::<f32>();

        assert_eq!(offset_sum, Vec2::ZERO);
        assert!((weight_sum - 1.0).abs() < f32::EPSILON);
        assert!(samples.iter().all(|sample| sample.z == 0.25));
    }

    #[test]
    fn single_pass_uses_center_sample_at_full_weight() {
        assert_eq!(sample_parameters(0, 1), Vec4::new(0.0, 0.0, 1.0, 0.0));
        for index in 1..4 {
            assert_eq!(sample_parameters(index, 1).z, 0.0);
        }
    }
}
