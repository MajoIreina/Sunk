use bevy::prelude::Resource;

use std::time::Duration;

pub(crate) const LENS_INFLUENCE_SCALE_MIN: f32 = 0.40;
pub(crate) const LENS_INFLUENCE_SCALE_DEFAULT: f32 = 1.00;
// (11.5 Rs disk radius * 1.10 visible padding) /
// (2.598076 critical impact * 3.45 baseline influence radii) = 1.4113.
// Staying just below that value guarantees the lens never crosses the visible
// left/right edge of the accretion disk despite floating-point roundoff.
pub(crate) const LENS_INFLUENCE_SCALE_MAX: f32 = 1.41;

/// User-selectable update and render cadence for the desktop effect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameRateLimit {
    Fps30,
    #[default]
    Fps60,
    Fps120,
}

impl FrameRateLimit {
    pub const ALL: [Self; 3] = [Self::Fps30, Self::Fps60, Self::Fps120];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fps30 => "30 FPS",
            Self::Fps60 => "60 FPS（推荐）",
            Self::Fps120 => "120 FPS",
        }
    }

    pub fn frame_time(self) -> Duration {
        Duration::from_secs_f64(match self {
            Self::Fps30 => 1.0 / 30.0,
            Self::Fps60 => 1.0 / 60.0,
            Self::Fps120 => 1.0 / 120.0,
        })
    }
}

/// User-facing quality presets shared by the ray integrator and desktop capture.
///
/// The values intentionally live outside the shader material. This keeps the
/// settings window stable if the GPU uniform layout changes later.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RenderQuality {
    Performance,
    #[default]
    Balanced,
    Cinematic,
}

impl RenderQuality {
    pub const ALL: [Self; 3] = [Self::Performance, Self::Balanced, Self::Cinematic];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Performance => "性能优先",
            Self::Balanced => "均衡",
            Self::Cinematic => "电影级",
        }
    }

    /// Maximum number of adaptive geodesic integration steps per pixel.
    pub const fn integration_steps(self) -> u32 {
        match self {
            Self::Performance => 128,
            Self::Balanced => 256,
            Self::Cinematic => 384,
        }
    }

    /// Integrator selector passed to the shader: `0` chooses adaptive midpoint
    /// and `1` chooses adaptive RK4.
    pub const fn integration_quality(self) -> f32 {
        match self {
            Self::Performance => 0.0,
            Self::Balanced | Self::Cinematic => 1.0,
        }
    }

    /// Error tolerance used when a ray turns close to the photon sphere.
    pub const fn turn_tolerance(self) -> f32 {
        match self {
            Self::Performance => 0.060,
            Self::Balanced => 0.035,
            Self::Cinematic => 0.018,
        }
    }
}

/// Anti-aliasing strategies that are valid for the premultiplied-alpha,
/// full-screen ray-marched renderer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AntiAliasingMode {
    Off,
    /// Bevy's morphological post-process blends all RGBA channels together,
    /// preserving the premultiplied-alpha relationship at transparent edges.
    #[default]
    Smaa,
    /// Four independent geodesic rays at deterministic 2x2 subpixel offsets.
    Ssaa2x2,
}

impl AntiAliasingMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Smaa, Self::Ssaa2x2];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "关闭",
            Self::Smaa => "SMAA（推荐）",
            Self::Ssaa2x2 => "SSAA 2x2（高开销）",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Off => "不执行额外抗锯齿，性能最好，但黑洞轮廓可能出现锯齿。",
            Self::Smaa => "使用形态学边缘检测平滑整幅画面，透明边缘稳定且开销适中。",
            Self::Ssaa2x2 => "每个像素追踪四条子像素光线，细节最准确，但 GPU 开销约为四倍。",
        }
    }

    pub const fn spatial_samples(self) -> u32 {
        match self {
            Self::Off | Self::Smaa => 1,
            Self::Ssaa2x2 => 4,
        }
    }

    pub const fn uses_smaa(self) -> bool {
        matches!(self, Self::Smaa)
    }
}

/// Single source of truth for all user-adjustable black-hole appearance values.
///
/// Colors are stored in linear RGB because the renderer accumulates linear
/// radiance. UI code and future persistence code should edit this resource,
/// while the rendering layer converts it to its current uniform representation.
#[derive(Resource, Debug, Clone)]
pub struct BlackHoleSettings {
    /// Apparent angular size relative to the default composition.
    pub apparent_size: f32,
    /// Artistic multiplier applied after the physical black-body disk color.
    pub disk_tint_linear: [f32; 3],
    /// Simulation time multiplier. Zero freezes disk motion.
    pub animation_speed: f32,
    /// Reference temperature at the hot inner edge of the accretion disk.
    pub disk_temperature_kelvin: f32,
    /// Optical-depth multiplier for the volumetric disk material.
    pub disk_density: f32,
    /// Vertical scale multiplier for the disk atmosphere.
    pub disk_thickness: f32,
    /// Radiance multiplier before exposure and tone mapping.
    pub emission_strength: f32,
    /// Amount of escaped-ray displacement applied to the captured desktop.
    pub background_warp: f32,
    /// Dimensionless multiplier applied to the physically derived lens extent.
    /// Its maximum aligns with the visible horizontal edge of the accretion disk.
    pub lens_radius: f32,
    /// Optical depth of the faint hot corona around the disk.
    pub corona_opacity: f32,
    /// Strength of multi-scale density variation in the disk material.
    pub turbulence: f32,
    /// Blend between a thin photosphere and the flowing volumetric cloud layer.
    pub cloudiness: f32,
    /// Final scene exposure multiplier.
    pub exposure: f32,
    pub render_quality: RenderQuality,
    pub anti_aliasing: AntiAliasingMode,
    pub frame_rate_limit: FrameRateLimit,
}

impl Default for BlackHoleSettings {
    fn default() -> Self {
        Self {
            apparent_size: 1.0,
            disk_tint_linear: [1.0, 1.0, 1.0],
            animation_speed: 1.0,
            disk_temperature_kelvin: 10_500.0,
            disk_density: 2.4,
            disk_thickness: 1.0,
            emission_strength: 1.8,
            background_warp: 1.0,
            lens_radius: LENS_INFLUENCE_SCALE_DEFAULT,
            corona_opacity: 0.55,
            turbulence: 0.72,
            cloudiness: 0.78,
            exposure: 1.0,
            render_quality: RenderQuality::Balanced,
            anti_aliasing: AntiAliasingMode::Smaa,
            frame_rate_limit: FrameRateLimit::Fps60,
        }
    }
}

impl BlackHoleSettings {
    /// Keeps values valid when they originate outside the built-in UI, such as
    /// a future settings file or command-line override.
    pub fn sanitize(&mut self) {
        self.apparent_size = finite_or(self.apparent_size, 1.0).clamp(0.25, 3.0);
        self.animation_speed = finite_or(self.animation_speed, 1.0).clamp(0.0, 4.0);
        self.disk_temperature_kelvin =
            finite_or(self.disk_temperature_kelvin, 10_500.0).clamp(2_000.0, 25_000.0);
        self.disk_density = finite_or(self.disk_density, 2.4).clamp(0.25, 16.0);
        self.disk_thickness = finite_or(self.disk_thickness, 1.0).clamp(0.25, 3.0);
        self.emission_strength = finite_or(self.emission_strength, 1.8).clamp(0.1, 10.0);
        self.background_warp = finite_or(self.background_warp, 1.0).clamp(0.0, 1.5);
        self.lens_radius = clamped_lens_influence_scale(self.lens_radius);
        self.corona_opacity = finite_or(self.corona_opacity, 0.55).clamp(0.0, 1.5);
        self.turbulence = finite_or(self.turbulence, 0.72).clamp(0.0, 1.0);
        self.cloudiness = finite_or(self.cloudiness, 0.78).clamp(0.0, 1.0);
        self.exposure = finite_or(self.exposure, 1.0).clamp(0.25, 4.0);

        for channel in &mut self.disk_tint_linear {
            *channel = finite_or(*channel, 1.0).clamp(0.0, 1.0);
        }
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

pub(crate) fn clamped_lens_influence_scale(value: f32) -> f32 {
    finite_or(value, LENS_INFLUENCE_SCALE_DEFAULT)
        .clamp(LENS_INFLUENCE_SCALE_MIN, LENS_INFLUENCE_SCALE_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_presets_are_ordered_by_cost() {
        assert!(
            RenderQuality::Performance.integration_steps()
                < RenderQuality::Balanced.integration_steps()
        );
        assert!(
            RenderQuality::Balanced.integration_steps()
                < RenderQuality::Cinematic.integration_steps()
        );
        assert_eq!(RenderQuality::Performance.integration_quality(), 0.0);
        assert_eq!(RenderQuality::Balanced.integration_quality(), 1.0);
        assert!(
            RenderQuality::Performance.turn_tolerance() > RenderQuality::Cinematic.turn_tolerance()
        );
    }

    #[test]
    fn anti_aliasing_modes_expose_only_real_render_paths() {
        assert_eq!(AntiAliasingMode::ALL.len(), 3);
        assert_eq!(AntiAliasingMode::Off.spatial_samples(), 1);
        assert_eq!(AntiAliasingMode::Smaa.spatial_samples(), 1);
        assert_eq!(AntiAliasingMode::Ssaa2x2.spatial_samples(), 4);
        assert!(AntiAliasingMode::Smaa.uses_smaa());
        assert!(!AntiAliasingMode::Ssaa2x2.uses_smaa());
        assert_eq!(
            BlackHoleSettings::default().anti_aliasing,
            AntiAliasingMode::Smaa
        );
    }

    #[test]
    fn frame_rate_options_are_exactly_the_three_supported_tiers() {
        assert_eq!(FrameRateLimit::ALL.len(), 3);
        assert_eq!(FrameRateLimit::Fps30.frame_time().as_nanos(), 33_333_333);
        assert_eq!(FrameRateLimit::Fps60.frame_time().as_nanos(), 16_666_667);
        assert_eq!(FrameRateLimit::Fps120.frame_time().as_nanos(), 8_333_333);
        assert_eq!(
            BlackHoleSettings::default().frame_rate_limit,
            FrameRateLimit::Fps60
        );
    }

    #[test]
    fn sanitize_recovers_non_finite_and_out_of_range_values() {
        let mut settings = BlackHoleSettings {
            apparent_size: f32::NAN,
            animation_speed: f32::INFINITY,
            cloudiness: f32::NEG_INFINITY,
            lens_radius: 99.0,
            disk_tint_linear: [-1.0, 0.5, 4.0],
            ..Default::default()
        };

        settings.sanitize();

        assert_eq!(settings.apparent_size, 1.0);
        assert_eq!(settings.animation_speed, 1.0);
        assert_eq!(settings.cloudiness, 0.78);
        assert_eq!(settings.lens_radius, LENS_INFLUENCE_SCALE_MAX);
        assert_eq!(settings.disk_tint_linear, [0.0, 0.5, 1.0]);
    }

    #[test]
    fn lens_influence_scale_recovers_invalid_values_and_respects_disk_edge_limit() {
        assert_eq!(
            clamped_lens_influence_scale(f32::NAN),
            LENS_INFLUENCE_SCALE_DEFAULT
        );
        assert_eq!(
            clamped_lens_influence_scale(-10.0),
            LENS_INFLUENCE_SCALE_MIN
        );
        assert_eq!(clamped_lens_influence_scale(10.0), LENS_INFLUENCE_SCALE_MAX);
    }
}
