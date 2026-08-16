#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct BlackHoleUniform {
    viewport_time_fov: vec4<f32>,
    camera: vec4<f32>,
    disk: vec4<f32>,
    integration: vec4<f32>,
    appearance: vec4<f32>,
    material: vec4<f32>,
    tint: vec4<f32>,
    desktop: vec4<f32>,
    desktop_uv_origin_scale: vec4<f32>,
    sample: vec4<f32>,
    drag_feedback: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> params: BlackHoleUniform;

@group(#{MATERIAL_BIND_GROUP}) @binding(1)
var desktop_texture: texture_2d<f32>;

@group(#{MATERIAL_BIND_GROUP}) @binding(2)
var desktop_sampler: sampler;

const PI: f32 = 3.141592653589793;
const CRITICAL_IMPACT_PARAMETER_SQUARED: f32 = 6.75;
const MAX_TRACE_STEPS: u32 = 384u;
const LENS_ALPHA_FEATHER_PIXELS: f32 = 12.0;
const LENS_UV_RETURN_PIXELS: f32 = 28.0;

struct RayState {
    position: vec3<f32>,
    velocity: vec3<f32>,
};

struct RayResult {
    radiance: vec3<f32>,
    transmittance: f32,
    exit_position: vec3<f32>,
    exit_direction: vec3<f32>,
    captured: f32,
    escaped: f32,
    impact_parameter: f32,
    closest_radius: f32,
    path_length: f32,
};

struct OpticalSample {
    source: vec3<f32>,
    opacity: f32,
};

struct DiskCloudPattern {
    density: f32,
    heat: f32,
    filament: f32,
    photosphere: f32,
};

struct TransferState {
    radiance: vec3<f32>,
    transmittance: f32,
};

struct BackgroundProjection {
    uv: vec2<f32>,
    coverage: f32,
};

fn smootherstep01(value: f32) -> f32 {
    let t = clamp(value, 0.0, 1.0);
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn safe_normalize(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    // Scaling first avoids both overflow in dot(v, v) and Inf * 0 when a
    // malformed intermediate vector reaches the normalizer. Comparisons with
    // NaN are false, so the first condition also rejects NaNs.
    let largest = max(max(abs(v.x), abs(v.y)), abs(v.z));
    if (!(largest > 1.0e-20) || largest > 1.0e18) {
        return fallback;
    }

    let scaled = v / largest;
    let length_squared = dot(scaled, scaled);
    if (!(length_squared > 1.0e-12)) {
        return fallback;
    }
    return scaled * inverseSqrt(length_squared);
}

fn hash12(p: vec2<f32>) -> f32 {
    let p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    let q = p3 + dot(p3, p3.yzx + vec3<f32>(33.33));
    return fract((q.x + q.y) * q.z);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let local = fract(p);
    let smooth_local = local * local * (vec2<f32>(3.0) - 2.0 * local);
    let a = hash12(cell);
    let b = hash12(cell + vec2<f32>(1.0, 0.0));
    let c = hash12(cell + vec2<f32>(0.0, 1.0));
    let d = hash12(cell + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, smooth_local.x), mix(c, d, smooth_local.x), smooth_local.y);
}

fn rotate_and_scale(p: vec2<f32>, scale: f32) -> vec2<f32> {
    // Irrational-looking octave transforms suppress axis-aligned value-noise
    // artifacts without evaluating trigonometry for every octave.
    return vec2<f32>(
        0.8 * p.x + 0.6 * p.y,
        -0.6 * p.x + 0.8 * p.y,
    ) * scale;
}

fn cloud_fbm(p: vec2<f32>) -> f32 {
    // Three octaves are enough after domain warping. This function is evaluated
    // inside the geodesic loop, so an unbounded/high-octave fBm is prohibitively
    // expensive at desktop-window resolutions.
    let octave0 = value_noise(p);
    let p1 = rotate_and_scale(p, 2.03) + vec2<f32>(11.3, -7.1);
    let octave1 = value_noise(p1);
    let p2 = rotate_and_scale(p1, 2.01) + vec2<f32>(-3.7, 19.1);
    let octave2 = value_noise(p2);
    return octave0 * 0.55 + octave1 * 0.29 + octave2 * 0.16;
}

fn animation_time() -> f32 {
    // Do not hard-wrap time: Keplerian angular velocity differs at every radius,
    // so no single reset period can preserve all phases and a wrap would make
    // the entire disk visibly jump. f32 retains sub-frame-enough precision for
    // normal multi-hour desktop sessions.
    return max(params.viewport_time_fov.z, 0.0);
}

fn co_rotating_disk_domain(
    radius: f32,
    advected_phi: f32,
    layer_coordinate: f32,
    inner: f32,
) -> vec2<f32> {
    let orbital_direction = vec2<f32>(cos(advected_phi), sin(advected_phi));
    let log_radius = log(max(radius / inner, 0.36));
    return orbital_direction * radius * 0.82
        + vec2<f32>(0.68, -0.43) * log_radius
        + vec2<f32>(0.31, -0.24) * layer_coordinate;
}

fn radial_window(radius: f32) -> f32 {
    let inner = max(params.disk.x, 1.01);
    let outer = max(params.disk.y, inner + 0.1);
    let horizon = max(params.integration.x, 1.0);
    let span = max(outer - inner, 0.1);
    // The photosphere is not clipped at an infinitely thin ISCO/outer rim. Its
    // optical depth reaches zero with a zero derivative over a visibly broad
    // interval, while a tenuous plunging component bridges the ISCO to the hole.
    let inner_width = max(0.28, min(inner * 0.18, span * 0.18));
    let outer_width = max(0.68, span * 0.22);
    let inner_fade = smoothstep(inner - inner_width, inner + inner_width, radius);
    let outer_fade = 1.0 - smoothstep(outer - outer_width, outer, radius);
    let main_disk = inner_fade * outer_fade;

    let plunge_start = horizon + max(0.035, horizon * 0.045);
    let plunge_gate = smoothstep(plunge_start, inner - inner_width * 0.20, radius)
        * (1.0 - inner_fade);
    let plunge_decay = exp(
        -1.72 * max(inner - radius, 0.0) / max(inner - horizon, 0.1),
    );
    let plunging_atmosphere = 0.26 * plunge_gate * plunge_decay * outer_fade;
    return clamp(main_disk + plunging_atmosphere, 0.0, 1.0);
}

fn disk_cloud_pattern(position: vec3<f32>, radius: f32) -> DiskCloudPattern {
    let inner = max(params.disk.x, 1.01);
    let outer = max(params.disk.y, inner + 0.1);
    let radial_span = max(outer - inner, 0.1);
    let radial_coordinate = clamp((radius - inner) / radial_span, -0.25, 1.25);
    let half_thickness = max(params.disk.z, 0.01);
    let layer_coordinate = clamp(position.y / half_thickness, -4.0, 4.0);
    let absolute_layer = abs(layer_coordinate);
    let turbulence = clamp(params.material.z, 0.0, 1.0);
    let cloudiness = clamp(params.material.w, 0.0, 1.0);
    let phi = atan2(position.z, position.x);
    let omega = sqrt(0.5 / max(radius * radius * radius, 1.0e-5));
    let flow_time = animation_time() * params.disk.w;

    // Keplerian omega ~ r^-3/2 naturally shears the pattern. The weak
    // height-dependent multiplier lets the upper and lower atmosphere slide
    // past the photosphere instead of looking like an extruded 2-D texture.
    let layer_angular_scale = 1.0 + 0.038 * layer_coordinate;
    let advected_phi = phi - flow_time * omega * layer_angular_scale;
    // Build the domain from a co-rotating Cartesian point. Unlike sampling phi
    // directly, this remains continuous across the atan2 seam. Integer-armed
    // spiral phases below are periodic at the same seam as well.
    let domain = co_rotating_disk_domain(radius, advected_phi, layer_coordinate, inner);

    // Low-frequency vector noise bends the higher-frequency cloud field. This
    // domain warp is what changes soft blobs into connected rolling filaments.
    let warp_domain = rotate_and_scale(domain, 0.34);
    let shared_macro = value_noise(warp_domain + vec2<f32>(12.7, -4.3));
    let warp = vec2<f32>(
        shared_macro,
        value_noise(rotate_and_scale(warp_domain, 1.07) + vec2<f32>(-8.1, 15.9)),
    ) * 2.0 - vec2<f32>(1.0);
    let warp_strength = mix(0.22, 1.35, turbulence) * mix(0.4, 1.0, cloudiness);
    let warped_domain = domain + warp * warp_strength;

    let macro_cloud = mix(shared_macro, cloud_fbm(warped_domain * 0.72), 0.78);
    let detail0 = value_noise(
        rotate_and_scale(warped_domain, 1.91)
            + vec2<f32>(17.2, -9.4)
            + vec2<f32>(0.19, -0.13) * layer_coordinate,
    );
    let detail1 = value_noise(
        rotate_and_scale(warped_domain, 3.83) + vec2<f32>(-6.4, 23.7),
    );
    let detail = mix(detail0, detail1, 0.36 + 0.34 * turbulence);

    // A broad radial carrier keeps the clouds tangential, but the warped FBM and
    // a co-rotating arc gate break it into soft, unequal streams. This avoids the
    // regular white contour bands produced by a dominant high-frequency cosine.
    let ring_phase = radial_coordinate * (10.0 * PI)
        + 0.96 * sin(2.0 * advected_phi + macro_cloud * 2.6)
        + (macro_cloud - 0.5) * mix(2.2, 5.0, turbulence)
        + (detail0 - detail1) * 2.1
        + 0.24 * layer_coordinate;
    let ring_wave = 0.5 + 0.5 * cos(ring_phase);
    let soft_ring = ring_wave * (0.64 + 0.36 * ring_wave);
    let arc_wave = 0.5 + 0.5 * sin(
        3.0 * advected_phi + detail0 * 2.8 - detail1 * 1.6,
    );
    let arc_envelope = smoothstep(
        0.30,
        0.76,
        0.50 * macro_cloud + 0.30 * detail0 + 0.20 * arc_wave,
    );
    let tangential_filament = clamp(
        soft_ring * mix(0.10, 1.0, arc_envelope),
        0.0,
        1.0,
    );

    // FBM and octave interference carry most of the optical variation. The
    // tangential carrier is deliberately secondary, so it reads as orbiting
    // cloud filaments rather than a stack of luminous geometric rings.
    let shear = clamp(0.35 + 0.40 * detail + 0.25 * macro_cloud, 0.0, 1.0);
    let noise_filament = 1.0 - abs(detail * 2.0 - 1.0);
    let cloud_field = macro_cloud * 0.48
        + noise_filament * 0.22
        + tangential_filament * 0.20
        + shear * 0.10;

    // Raising the threshold with cloudiness opens transparent lanes between
    // dense clumps. A small wisp term preserves tenuous material around them.
    let threshold = mix(0.34, 0.55, cloudiness);
    let clump = smoothstep(threshold - 0.20, threshold + 0.15, cloud_field);
    let wisps = smoothstep(0.47, 0.84, detail) * (0.45 + 0.55 * macro_cloud);
    let broad_cloud = 0.76 + 0.30 * clamp(
        0.62 * macro_cloud + 0.24 * detail + 0.14 * soft_ring,
        0.0,
        1.0,
    );
    let cloud_density = 0.018
        + 1.18 * clump * (0.52 + 0.52 * macro_cloud + 0.18 * tangential_filament)
            * broad_cloud
        + 0.28 * wisps;
    let density = max(mix(1.0, cloud_density, cloudiness), 0.002);

    // The emitting photosphere sits above and below the cooler dense mid-plane.
    // The explicit vertical temperature profile remains visible even when cloud
    // density is high, creating depth instead of a single flat emissive sheet.
    let photosphere_offset = (absolute_layer - 0.72) / 0.46;
    let photosphere = exp(-photosphere_offset * photosphere_offset);
    let vertical_heat = 0.80 + 0.30 * photosphere
        - 0.09 * smoothstep(1.45, 3.6, absolute_layer);
    let cloud_heat = clamp(
        (0.54 + 0.50 * macro_cloud + 0.12 * tangential_filament + 0.10 * detail)
            * vertical_heat,
        0.42,
        1.38,
    );
    let heat = mix(1.0, cloud_heat, cloudiness);
    return DiskCloudPattern(density, heat, tangential_filament, photosphere);
}

fn disk_volume_pattern(position: vec3<f32>, radius: f32) -> DiskCloudPattern {
    // The atmosphere is sampled on every geodesic step, so use a lower-frequency
    // advected field here. The photosphere keeps the richer warped FBM above,
    // while these two noise bands provide soft volumetric depth at a stable cost.
    let inner = max(params.disk.x, 1.01);
    let outer = max(params.disk.y, inner + 0.1);
    let radial_coordinate = clamp((radius - inner) / max(outer - inner, 0.1), -0.25, 1.25);
    let half_thickness = max(params.disk.z, 0.01);
    let layer_coordinate = clamp(position.y / half_thickness, -4.0, 4.0);
    let absolute_layer = abs(layer_coordinate);
    let turbulence = clamp(params.material.z, 0.0, 1.0);
    let cloudiness = clamp(params.material.w, 0.0, 1.0);
    let phi = atan2(position.z, position.x);
    let omega = sqrt(0.5 / max(radius * radius * radius, 1.0e-5));
    let advected_phi = phi
        - animation_time() * params.disk.w * omega * (1.0 + 0.038 * layer_coordinate);
    let domain = co_rotating_disk_domain(radius, advected_phi, layer_coordinate, inner);
    let warp_domain = rotate_and_scale(domain, 0.34);
    let shared_macro = value_noise(warp_domain + vec2<f32>(12.7, -4.3));
    let macro_cloud = mix(
        shared_macro,
        value_noise(domain * 0.78 + vec2<f32>(4.7, -8.2)),
        0.34,
    );
    let detail = value_noise(
        rotate_and_scale(domain, 1.83) + vec2<f32>(-11.4, 6.8),
    );

    let ring_phase = radial_coordinate * (8.0 * PI)
        + 0.82 * sin(2.0 * advected_phi + macro_cloud * 2.2)
        + (macro_cloud - 0.5) * mix(1.8, 4.0, turbulence)
        + (detail - 0.5) * 1.4
        + 0.24 * layer_coordinate;
    let ring_wave = 0.5 + 0.5 * cos(ring_phase);
    let soft_ring = ring_wave * (0.68 + 0.32 * ring_wave);
    let arc_wave = 0.5 + 0.5 * sin(3.0 * advected_phi + detail * 2.4);
    let arc_envelope = smoothstep(
        0.30,
        0.76,
        0.58 * macro_cloud + 0.24 * detail + 0.18 * arc_wave,
    );
    let tangential_filament = clamp(
        soft_ring * mix(0.08, 0.78, arc_envelope),
        0.0,
        1.0,
    );
    let density = max(
        mix(
            1.0,
            0.12 + 0.82 * macro_cloud + 0.30 * detail + 0.14 * tangential_filament,
            cloudiness,
        ),
        0.002,
    );
    let photosphere_offset = (absolute_layer - 0.72) / 0.48;
    let photosphere = exp(-photosphere_offset * photosphere_offset);
    let heat = mix(
        1.0,
        clamp(
            (0.64 + 0.34 * macro_cloud + 0.08 * tangential_filament)
                * (0.84 + 0.24 * photosphere),
            0.48,
            1.32,
        ),
        cloudiness,
    );
    return DiskCloudPattern(density, heat, tangential_filament, photosphere);
}

fn disk_temperature(radius: f32, pattern: DiskCloudPattern) -> f32 {
    // Newtonian zero-torque thin-disk flux, normalized so material.x is the
    // actual peak effective temperature. For a Schwarzschild hole r_ISCO=3 r_s.
    let inner = max(params.disk.x, 1.01);
    let x = max(radius / inner, 1.00001);
    let flux_shape = 1.0 / (x * x * x) * max(1.0 - inverseSqrt(x), 0.0);
    let peak_flux_shape = pow(36.0 / 49.0, 3.0) / 7.0;
    let thin_disk_temperature = pow(max(flux_shape / peak_flux_shape, 0.0), 0.25);
    // Optically thin plunging gas does not terminate at an artificial black ring.
    // This low-energy bridge vanishes at the horizon and merges into the regular
    // thin-disk temperature over a finite radial interval.
    let horizon = max(params.integration.x, 1.0);
    let plunge_temperature = 0.39
        * smoothstep(horizon * 1.035, horizon * 1.42, radius)
        * (1.0 - smoothstep(inner + 0.10, inner + max(0.55, inner * 0.24), radius));
    let thin_temperature_squared = thin_disk_temperature * thin_disk_temperature;
    let plunge_temperature_squared = plunge_temperature * plunge_temperature;
    let normalized_temperature = pow(
        thin_temperature_squared * thin_temperature_squared
            + plunge_temperature_squared * plunge_temperature_squared,
        0.25,
    );
    let peak_kelvin = clamp(params.material.x, 1000.0, 40000.0);
    let cloudiness = clamp(params.material.w, 0.0, 1.0);
    let cloud_temperature = clamp(
        0.62 + 0.38 * pattern.heat + 0.16 * sqrt(max(pattern.density, 0.0)),
        0.62,
        1.30,
    );
    let temperature_variation = mix(1.0, cloud_temperature, cloudiness);
    return peak_kelvin * normalized_temperature * temperature_variation;
}

fn blackbody_chroma(kelvin: f32) -> vec3<f32> {
    // Planckian-locus xy approximation followed by xyY -> XYZ -> linear sRGB.
    // It supplies chromaticity only; radiometric strength is applied separately.
    let temperature = clamp(kelvin, 1667.0, 25000.0);
    let temperature_squared = temperature * temperature;
    let temperature_cubed = temperature_squared * temperature;

    var x = 0.0;
    if temperature <= 4000.0 {
        x = -0.2661239e9 / temperature_cubed - 0.2343580e6 / temperature_squared
            + 0.8776956e3 / temperature + 0.179910;
    } else {
        x = -3.0258469e9 / temperature_cubed + 2.1070379e6 / temperature_squared
            + 0.2226347e3 / temperature + 0.240390;
    }

    var y = 0.0;
    if temperature <= 2222.0 {
        y = -1.1063814 * x * x * x - 1.3481102 * x * x + 2.1855583 * x - 0.20219683;
    } else if temperature <= 4000.0 {
        y = -0.9549476 * x * x * x - 1.3741859 * x * x + 2.0913702 * x - 0.16748867;
    } else {
        y = 3.081758 * x * x * x - 5.8733867 * x * x + 3.75113 * x - 0.37001483;
    }

    let inverse_y = 1.0 / max(y, 1.0e-4);
    let xyz = vec3<f32>(x * inverse_y, 1.0, max(1.0 - x - y, 0.0) * inverse_y);
    var rgb = max(
        vec3<f32>(
            3.2406 * xyz.x - 1.5372 * xyz.y - 0.4986 * xyz.z,
            -0.9689 * xyz.x + 1.8758 * xyz.y + 0.0415 * xyz.z,
            0.0557 * xyz.x - 0.2040 * xyz.y + 1.0570 * xyz.z,
        ),
        vec3<f32>(0.0),
    );
    let luminance = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    rgb /= max(luminance, 1.0e-4);
    return rgb;
}

fn circular_frequency_shift(radius: f32, photon_lambda: f32, observer_radius: f32) -> f32 {
    // g = nu_observed / nu_emitted for a circular Schwarzschild emitter and a
    // static finite-radius observer. photon_lambda is L_axis/E for the physical
    // photon; the renderer traces the opposite direction, hence its sign is fixed
    // once at ray initialization.
    let omega = sqrt(0.5 / max(radius * radius * radius, 1.0e-6));
    let emitter_clock = sqrt(max(1.0 - 1.5 / radius, 1.0e-5));
    let observer_clock = sqrt(max(1.0 - 1.0 / max(observer_radius, 1.001), 1.0e-5));
    let longitudinal = max(1.0 - omega * photon_lambda, 0.05);
    return clamp(emitter_clock / (observer_clock * longitudinal), 0.08, 4.0);
}

fn observed_frequency_shift(radius: f32, photon_lambda: f32, observer_radius: f32) -> f32 {
    let inner = max(params.disk.x, 1.51);
    let horizon = max(params.integration.x, 1.0);
    let transition_inner = max(horizon * 1.12, inner - max(0.25, inner * 0.12));
    let orbit_radius = max(radius, transition_inner);
    let circular = circular_frequency_shift(orbit_radius, photon_lambda, observer_radius);
    if radius >= inner {
        return circular;
    }

    // Material inside the ISCO is no longer on a stable circular orbit. Match
    // the circular solution exactly at the ISCO, then use the Schwarzschild
    // lapse and a modest radial-infall Doppler term so radiation approaches zero
    // continuously at the horizon. Smoothstep has zero endpoint derivatives,
    // making both joins C1 rather than a visible brightness/color kink.
    let inner_shift = circular_frequency_shift(inner, photon_lambda, observer_radius);
    let safe_radius = max(radius, horizon + 1.0e-4);
    let lapse_ratio = sqrt(clamp(
        (1.0 - horizon / safe_radius) / max(1.0 - horizon / inner, 1.0e-4),
        0.0,
        1.0,
    ));
    let plunge_progress = 1.0 - smoothstep(horizon * 1.035, inner, radius);
    let plunge = inner_shift * lapse_ratio * (1.0 - 0.34 * plunge_progress);
    let circular_blend = smoothstep(transition_inner, inner, radius);
    return clamp(mix(plunge, circular, circular_blend), 0.01, 4.0);
}

fn source_radiance(
    radius: f32,
    pattern: DiskCloudPattern,
    photon_lambda: f32,
    observer_radius: f32,
    limb_factor: f32,
) -> vec3<f32> {
    let emitted_temperature = disk_temperature(radius, pattern);
    let shift = observed_frequency_shift(radius, photon_lambda, observer_radius);
    let observed_temperature = max(emitted_temperature * shift, 1.0);
    let peak_temperature = max(params.material.x, 1000.0);
    let relative_temperature = clamp(observed_temperature / peak_temperature, 0.0, 4.0);
    let relative_temperature_squared = relative_temperature * relative_temperature;
    let bolometric_strength = min(
        relative_temperature_squared * relative_temperature_squared,
        128.0,
    );
    let tint = clamp(params.tint.rgb, vec3<f32>(0.0), vec3<f32>(8.0));
    let cloudiness = clamp(params.material.w, 0.0, 1.0);
    let cloud_emissivity = clamp(
        0.025 + pattern.density * (
            0.38 + 0.50 * pattern.heat + 0.10 * pattern.filament
        ) + 0.12 * pattern.photosphere * (0.62 + 0.38 * pattern.filament),
        0.025,
        1.65,
    );
    let emissivity = mix(1.0, cloud_emissivity, cloudiness);
    return blackbody_chroma(observed_temperature) * tint * bolometric_strength
        * limb_factor * emissivity * max(params.appearance.y, 0.0);
}

fn disk_surface_fraction(start: vec3<f32>, end: vec3<f32>) -> f32 {
    let denominator = start.y - end.y;
    if (abs(denominator) <= 1.0e-6) {
        return 2.0;
    }

    let segment_fraction = start.y / denominator;
    if (!(segment_fraction > 1.0e-5) || segment_fraction > 1.0) {
        return 2.0;
    }

    let hit = mix(start, end, segment_fraction);
    if (!(radial_window(length(hit.xz)) > 1.0e-5)) {
        return 2.0;
    }
    return segment_fraction;
}

fn sample_disk_surface(
    start: vec3<f32>,
    end: vec3<f32>,
    backward_direction: vec3<f32>,
    photon_lambda: f32,
    observer_radius: f32,
) -> OpticalSample {
    let segment_fraction = disk_surface_fraction(start, end);
    if segment_fraction > 1.0 {
        return OpticalSample(vec3<f32>(0.0), 0.0);
    }

    let hit = mix(start, end, segment_fraction);
    let radius = length(hit.xz);
    let radial_density = radial_window(radius);
    if (!(radial_density > 1.0e-5)) {
        return OpticalSample(vec3<f32>(0.0), 0.0);
    }

    let pattern = disk_cloud_pattern(hit, radius);
    let physical_photon_direction = -safe_normalize(backward_direction, vec3<f32>(0.0, 0.0, 1.0));
    let cosine_to_normal = max(abs(physical_photon_direction.y), 0.04);
    let cloudiness = clamp(params.material.w, 0.0, 1.0);
    let porous_density = mix(
        pattern.density,
        pow(max(pattern.density, 0.002), 1.45),
        cloudiness,
    );
    // A square-root grazing correction retains physical path-length behavior
    // without turning every low-density cloud lane fully opaque at shallow view
    // angles. The surrounding volume still supplies the missing long-path haze.
    let grazing_path = 1.0 / max(sqrt(cosine_to_normal), 0.24);
    let edge_depth = radial_density * (0.30 + 0.70 * radial_density);
    let optical_depth = max(params.appearance.x, 0.0) * edge_depth
        * porous_density * grazing_path * mix(0.76, 0.46, cloudiness);
    let opacity = 1.0 - exp(-min(optical_depth, 20.0));
    let limb_darkening = 0.5 + 0.75 * cosine_to_normal;
    let source = source_radiance(
        radius,
        pattern,
        photon_lambda,
        observer_radius,
        limb_darkening,
    );
    return OpticalSample(min(source, vec3<f32>(128.0)), clamp(opacity, 0.0, 1.0));
}

fn sample_corona(
    position: vec3<f32>,
    backward_direction: vec3<f32>,
    path_length: f32,
    photon_lambda: f32,
    observer_radius: f32,
) -> OpticalSample {
    let cloudiness = clamp(params.material.w, 0.0, 1.0);
    let cloud_volume_depth = max(params.appearance.x, 0.0) * 0.055 * cloudiness;
    let volume_depth = max(params.material.y, 0.0) + cloud_volume_depth;
    if (!(volume_depth > 1.0e-5) || !(path_length > 1.0e-6)) {
        return OpticalSample(vec3<f32>(0.0), 0.0);
    }

    let radius = length(position.xz);
    let radial_density = radial_window(radius);
    let half_thickness = max(params.disk.z, 0.01);
    if (!(radial_density > 1.0e-5) || abs(position.y) > half_thickness * 4.6) {
        return OpticalSample(vec3<f32>(0.0), 0.0);
    }

    let normalized_height = abs(position.y) / half_thickness;
    let gaussian_core = exp(-normalized_height * normalized_height * 1.85);
    let pattern = disk_volume_pattern(position, radius);
    let photosphere_offset = (normalized_height - 0.72) / 0.48;
    let photosphere_layer = exp(-photosphere_offset * photosphere_offset);
    let diffuse_halo = exp(-normalized_height * 1.06);
    let plume = diffuse_halo
        * smoothstep(0.035, 0.95, pattern.density);
    let layer_wave = 0.78 + 0.22 * (
        0.5 + 0.5 * sin(position.y / half_thickness * 3.4 + pattern.heat * 4.1)
    );
    let stratified_clear = 0.76 * gaussian_core + 0.24 * photosphere_layer;
    let stratified_cloud = 0.34 * gaussian_core
        + 0.43 * photosphere_layer * (0.82 + 0.18 * pattern.filament)
        + 0.23 * plume * layer_wave;
    let vertical_density = mix(stratified_clear, stratified_cloud, cloudiness);
    let clumped_density = mix(
        pattern.density,
        pow(max(pattern.density, 0.002), 1.22),
        cloudiness,
    );
    let edge_depth = radial_density * (0.34 + 0.66 * radial_density);
    let density = edge_depth * vertical_density * clumped_density;
    let optical_depth = volume_depth * density * path_length
        / max(half_thickness * 2.0, 0.05);
    let opacity = 1.0 - exp(-min(optical_depth, 20.0));
    let physical_photon_direction = -safe_normalize(backward_direction, vec3<f32>(0.0, 0.0, 1.0));
    let limb = 0.35 + 0.25 * abs(physical_photon_direction.y);
    let layer_emission = 0.18 + 0.30 * photosphere_layer + 0.07 * diffuse_halo;
    let volume_emission = layer_emission * mix(0.82, 1.08, cloudiness);
    let source = source_radiance(radius, pattern, photon_lambda, observer_radius, limb)
        * volume_emission;
    return OpticalSample(min(source, vec3<f32>(64.0)), clamp(opacity, 0.0, 1.0));
}

fn composite_optical_sample(transfer: TransferState, sample: OpticalSample) -> TransferState {
    let opacity = clamp(sample.opacity, 0.0, 1.0);
    let radiance = transfer.radiance + transfer.transmittance * opacity * sample.source;
    let transmittance = transfer.transmittance * (1.0 - opacity);
    return TransferState(radiance, transmittance);
}

fn schwarzschild_acceleration(position: vec3<f32>, h_squared: f32) -> vec3<f32> {
    let radius_squared = max(dot(position, position), 1.0e-8);
    let inverse_radius = inverseSqrt(radius_squared);
    let inverse_radius_squared = 1.0 / radius_squared;
    let inverse_radius_fifth = inverse_radius * inverse_radius_squared * inverse_radius_squared;
    return -1.5 * h_squared * position * inverse_radius_fifth;
}

fn ray_derivative(state: RayState, h_squared: f32) -> RayState {
    return RayState(state.velocity, schwarzschild_acceleration(state.position, h_squared));
}

fn midpoint_step(state: RayState, h_squared: f32, step_size: f32) -> RayState {
    let initial_acceleration = schwarzschild_acceleration(state.position, h_squared);
    let midpoint = RayState(
        state.position + state.velocity * (0.5 * step_size),
        state.velocity + initial_acceleration * (0.5 * step_size),
    );
    return RayState(
        state.position + midpoint.velocity * step_size,
        state.velocity + schwarzschild_acceleration(midpoint.position, h_squared) * step_size,
    );
}

fn rk4_step(state: RayState, h_squared: f32, step_size: f32) -> RayState {
    let k1 = ray_derivative(state, h_squared);
    let k2 = ray_derivative(
        RayState(
            state.position + k1.position * (0.5 * step_size),
            state.velocity + k1.velocity * (0.5 * step_size),
        ),
        h_squared,
    );
    let k3 = ray_derivative(
        RayState(
            state.position + k2.position * (0.5 * step_size),
            state.velocity + k2.velocity * (0.5 * step_size),
        ),
        h_squared,
    );
    let k4 = ray_derivative(
        RayState(
            state.position + k3.position * step_size,
            state.velocity + k3.velocity * step_size,
        ),
        h_squared,
    );
    let weighted_position = k1.position + 2.0 * k2.position + 2.0 * k3.position + k4.position;
    let weighted_velocity = k1.velocity + 2.0 * k2.velocity + 2.0 * k3.velocity + k4.velocity;
    return RayState(
        state.position + weighted_position * (step_size / 6.0),
        state.velocity + weighted_velocity * (step_size / 6.0),
    );
}

fn adaptive_step_size(state: RayState, h_squared: f32) -> f32 {
    let horizon = max(params.integration.x, 0.01);
    let radius = max(length(state.position), horizon + 1.0e-5);
    let speed_squared = max(dot(state.velocity, state.velocity), 1.0e-8);
    let speed = sqrt(speed_squared);
    let acceleration = schwarzschild_acceleration(state.position, h_squared);
    let perpendicular_acceleration = acceleration
        - state.velocity * (dot(acceleration, state.velocity) / speed_squared);
    let turn_tolerance = clamp(params.integration.w, 0.015, 0.10);
    let turn_step = turn_tolerance * speed / max(length(perpendicular_acceleration), 1.0e-5);
    let geometry_step = 0.12 * radius / speed;

    let radial_direction = state.position / radius;
    let radial_speed = dot(state.velocity, radial_direction);
    var horizon_step = 2.5;
    if radial_speed < 0.0 {
        horizon_step = 0.25 * max(radius - horizon, 0.001) / max(-radial_speed, 0.05);
    }

    return clamp(min(min(turn_step, geometry_step), horizon_step), 0.002, 2.5);
}

fn segment_sphere_first_hit(start: vec3<f32>, end: vec3<f32>, radius: f32) -> f32 {
    let delta = end - start;
    let quadratic_a = dot(delta, delta);
    if (!(quadratic_a > 1.0e-12)) {
        return 2.0;
    }

    let quadratic_b = dot(start, delta);
    let quadratic_c = dot(start, start) - radius * radius;
    let discriminant = quadratic_b * quadratic_b - quadratic_a * quadratic_c;
    if discriminant < 0.0 {
        return 2.0;
    }

    let root = sqrt(max(discriminant, 0.0));
    let near_fraction = (-quadratic_b - root) / quadratic_a;
    if near_fraction >= 0.0 && near_fraction <= 1.0 {
        return near_fraction;
    }
    let far_fraction = (-quadratic_b + root) / quadratic_a;
    if far_fraction >= 0.0 && far_fraction <= 1.0 {
        return far_fraction;
    }
    return 2.0;
}

fn trace_black_hole(camera_position: vec3<f32>, initial_direction: vec3<f32>) -> RayResult {
    var state = RayState(camera_position, initial_direction);
    var transfer = TransferState(vec3<f32>(0.0), 1.0);
    var captured = 0.0;
    var escaped = 0.0;
    var closest_radius = length(camera_position);
    var traced_path_length = 0.0;

    let angular_momentum = cross(camera_position, initial_direction);
    let h_squared = dot(angular_momentum, angular_momentum);
    let initial_radius = max(length(camera_position), 1.001);
    let orbit_energy = 0.5 * dot(initial_direction, initial_direction)
        - 0.5 * h_squared / (initial_radius * initial_radius * initial_radius);
    let asymptotic_energy_scale = sqrt(max(2.0 * orbit_energy, 1.0e-6));
    let impact_parameter = sqrt(max(h_squared, 0.0)) / asymptotic_energy_scale;
    // The shader follows the ray from observer to source. The physical photon's
    // angular momentum has the opposite sign.
    let photon_lambda = -angular_momentum.y / asymptotic_energy_scale;
    let escape_radius = max(max(params.disk.y * 4.0, params.camera.z * 2.0), 64.0);

    let step_limit = u32(clamp(params.integration.y, 1.0, f32(MAX_TRACE_STEPS)));
    var index = 0u;
    loop {
        if index >= MAX_TRACE_STEPS
            || index >= step_limit
            || transfer.transmittance < 0.002 {
            break;
        }

        let radius = length(state.position);
        closest_radius = min(closest_radius, radius);
        if radius <= params.integration.x {
            captured = 1.0;
            break;
        }
        if index > 4u && radius > escape_radius && dot(state.position, state.velocity) > 0.0 {
            escaped = 1.0;
            break;
        }

        let step_size = adaptive_step_size(state, h_squared);
        var next_state = midpoint_step(state, h_squared, step_size);
        if params.integration.z >= 0.5 {
            next_state = rk4_step(state, h_squared, step_size);
        }

        let horizon_fraction = segment_sphere_first_hit(
            state.position,
            next_state.position,
            params.integration.x,
        );
        var segment_end = next_state.position;
        if horizon_fraction <= 1.0 {
            segment_end = mix(state.position, next_state.position, horizon_fraction);
        }

        let segment = segment_end - state.position;
        let path_length = length(segment);
        traced_path_length += path_length;
        if path_length > 1.0e-6 {
            let segment_direction = safe_normalize(
                state.velocity + next_state.velocity,
                safe_normalize(segment, initial_direction),
            );
            let surface_fraction = disk_surface_fraction(state.position, segment_end);
            if surface_fraction <= 1.0 {
                // Integrate the atmosphere in strict front-to-back order. The
                // volume path is deliberately cheaper than the surface FBM, so
                // these explicit branches compile faster than a nested loop.
                let surface_position = mix(state.position, segment_end, surface_fraction);
                let before_surface = surface_position - state.position;
                let before_length = length(before_surface);
                if before_length > 1.0e-6 {
                    let corona_before = sample_corona(
                        state.position + before_surface * 0.5,
                        segment_direction,
                        before_length,
                        photon_lambda,
                        initial_radius,
                    );
                    transfer = composite_optical_sample(transfer, corona_before);
                }

                let surface = sample_disk_surface(
                    state.position,
                    segment_end,
                    segment_direction,
                    photon_lambda,
                    initial_radius,
                );
                transfer = composite_optical_sample(transfer, surface);

                let after_surface = segment_end - surface_position;
                let after_length = length(after_surface);
                if after_length > 1.0e-6 && transfer.transmittance >= 0.002 {
                    let corona_after = sample_corona(
                        surface_position + after_surface * 0.5,
                        segment_direction,
                        after_length,
                        photon_lambda,
                        initial_radius,
                    );
                    transfer = composite_optical_sample(transfer, corona_after);
                }
            } else {
                let corona = sample_corona(
                    state.position + segment * 0.5,
                    segment_direction,
                    path_length,
                    photon_lambda,
                    initial_radius,
                );
                transfer = composite_optical_sample(transfer, corona);
            }
        }

        state = next_state;
        if horizon_fraction <= 1.0 {
            state.position = segment_end;
            captured = 1.0;
            break;
        }
        index += 1u;
    }

    // A mathematically critical orbit requires infinitely many turns. If the
    // finite GPU budget expires, the conserved effective impact parameter gives
    // a stable capture classification instead of a frame-dependent dark speckle.
    if captured < 0.5 && escaped < 0.5 && transfer.transmittance >= 0.002 {
        let effective_impact_squared = h_squared / max(2.0 * orbit_energy, 1.0e-6);
        let initially_inward = dot(camera_position, initial_direction) < 0.0;
        if initially_inward && effective_impact_squared < CRITICAL_IMPACT_PARAMETER_SQUARED {
            captured = 1.0;
        } else {
            escaped = 1.0;
        }
    }

    return RayResult(
        transfer.radiance,
        transfer.transmittance,
        state.position,
        safe_normalize(state.velocity, initial_direction),
        captured,
        escaped,
        impact_parameter,
        closest_radius,
        traced_path_length,
    );
}

fn camera_basis() -> mat3x3<f32> {
    let yaw = params.camera.x;
    let pitch = params.camera.y;
    let forward = safe_normalize(
        -vec3<f32>(sin(yaw) * cos(pitch), sin(pitch), cos(yaw) * cos(pitch)),
        vec3<f32>(0.0, 0.0, -1.0),
    );
    let right = safe_normalize(
        cross(forward, vec3<f32>(0.0, 1.0, 0.0)),
        vec3<f32>(1.0, 0.0, 0.0),
    );
    let up = safe_normalize(cross(right, forward), vec3<f32>(0.0, 1.0, 0.0));
    return mat3x3<f32>(right, up, forward);
}

fn project_background_plane(
    exit_position: vec3<f32>,
    exit_direction: vec3<f32>,
    basis: mat3x3<f32>,
) -> BackgroundProjection {
    let right = basis[0];
    let up = basis[1];
    let forward = basis[2];
    let escape_radius = max(max(params.disk.y * 4.0, params.camera.z * 2.0), 64.0);
    let plane_depth = escape_radius * 1.25;
    let forward_speed = dot(exit_direction, forward);
    if (!(forward_speed > 1.0e-4)) {
        return BackgroundProjection(vec2<f32>(0.5), 0.0);
    }

    let distance_to_plane = (plane_depth - dot(exit_position, forward)) / forward_speed;
    if (!(distance_to_plane > 0.0) || distance_to_plane > 1.0e5) {
        return BackgroundProjection(vec2<f32>(0.5), 0.0);
    }

    let plane_hit = exit_position + exit_direction * distance_to_plane;
    let tan_half_fov = max(params.viewport_time_fov.w, 1.0e-4);
    let aspect = params.viewport_time_fov.x / max(params.viewport_time_fov.y, 1.0);
    let half_height = (params.camera.z + plane_depth) * tan_half_fov;
    let projected = vec2<f32>(
        0.5 + dot(plane_hit, right) / max(2.0 * half_height * aspect, 1.0e-4),
        0.5 - dot(plane_hit, up) / max(2.0 * half_height, 1.0e-4),
    );
    // Keep the unbounded coordinate. The caller interpolates the actual UV
    // displacement before deciding whether the finite desktop capture covers it.
    return BackgroundProjection(projected, 1.0);
}

fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((color * (a * color + vec3<f32>(b)))
        / (color * (c * color + vec3<f32>(d)) + vec3<f32>(e)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn tone_map_premultiplied(radiance: vec3<f32>, opacity: f32) -> vec3<f32> {
    if (!(opacity > 1.0e-5)) {
        return vec3<f32>(0.0);
    }
    let straight_hdr = clamp(radiance / opacity, vec3<f32>(0.0), vec3<f32>(128.0));
    return aces_fitted(straight_hdr * max(params.appearance.z, 0.0)) * opacity;
}

fn shade_sample(uv: vec2<f32>) -> vec4<f32> {
    let aspect = params.viewport_time_fov.x / max(params.viewport_time_fov.y, 1.0);
    let screen = vec2<f32>((uv.x * 2.0 - 1.0) * aspect, 1.0 - uv.y * 2.0);
    let basis = camera_basis();
    let camera_position = -basis[2] * params.camera.z;
    let initial_direction = safe_normalize(
        basis[2] + basis[0] * screen.x * params.viewport_time_fov.w
            + basis[1] * screen.y * params.viewport_time_fov.w,
        basis[2],
    );

    let ray = trace_black_hole(camera_position, initial_direction);
    let disk_opacity = clamp(1.0 - ray.transmittance, 0.0, 1.0);
    var premultiplied_rgb = tone_map_premultiplied(ray.radiance, disk_opacity);
    var alpha = disk_opacity;
    // Lens values are staged so the desktop remains a valid premultiplied layer
    // after the capture/escape branch converges.
    var lens_desktop_color = vec3<f32>(0.0);
    var lens_edge_coverage = 0.0;
    var lens_active = 0.0;

    if ray.captured > 0.5 {
        // Keep the disk's accumulated transmittance separate: setting it to zero
        // inside the integrator would make the adjustable horizon opacity inert.
        // Coverage is analytically softened over roughly one output pixel in
        // impact-parameter space, avoiding a binary black/transparent contour.
        let critical_impact = sqrt(CRITICAL_IMPACT_PARAMETER_SQUARED);
        let impact_per_pixel = 2.0 * params.camera.z
            * max(params.viewport_time_fov.w, 1.0e-4)
            / max(params.viewport_time_fov.y, 1.0);
        let shadow_feather = max(impact_per_pixel * 1.35, 0.003);
        let shadow_coverage = smoothstep(
            0.0,
            shadow_feather,
            max(critical_impact - ray.impact_parameter, 0.0),
        );
        alpha += ray.transmittance * clamp(params.appearance.w, 0.0, 1.0)
            * shadow_coverage;
    } else if ray.escaped > 0.5 && params.desktop.x > 0.5 {
        let projection = project_background_plane(ray.exit_position, ray.exit_direction, basis);
        let unbent_projection = project_background_plane(
            camera_position,
            initial_direction,
            basis,
        );
        let warp_amount = clamp(params.desktop.y, 0.0, 1.5);
        // Subtract the finite-plane projection of the original straight ray. The
        // remaining vector is the displacement produced by the integrated
        // geodesic itself, independent of camera projection conventions.
        let raw_deflection_uv = projection.uv - unbent_projection.uv;
        let viewport_size = max(params.viewport_time_fov.xy, vec2<f32>(1.0));
        let raw_warp_pixels = length(raw_deflection_uv * warp_amount * viewport_size);
        let critical_impact = sqrt(CRITICAL_IMPACT_PARAMETER_SQUARED);
        let normalized_impact = critical_impact
            / max(ray.impact_parameter, critical_impact);
        let impact_response = normalized_impact * normalized_impact;
        let normalized_closest = max(ray.closest_radius - critical_impact, 0.0)
            / critical_impact;
        let closest_response = 1.0
            / (1.0 + normalized_closest * normalized_closest);
        let direct_path_length = length(ray.exit_position - camera_position);
        let excess_path_length = max(ray.path_length - direct_path_length, 0.0);
        let path_response = 1.0 - exp(
            -excess_path_length / max(params.camera.z * 0.025, 0.15),
        );

        // desktop.z is the requested influence extent in normalized view-plane
        // units. Convert that extent back to the conserved impact parameter of
        // its boundary ray. This is a physical ray-space falloff, not a circular
        // UV mask, so the geodesic displacement and its asymmetric multiple
        // images remain untouched.
        let camera_radius = max(params.camera.z, 1.001);
        let influence_tangent = max(params.desktop.z, 0.0)
            * max(params.viewport_time_fov.w, 1.0e-4);
        let influence_h = camera_radius * influence_tangent
            * inverseSqrt(1.0 + influence_tangent * influence_tangent);
        let influence_energy_scale = sqrt(max(
            1.0 - influence_h * influence_h
                / (camera_radius * camera_radius * camera_radius),
            1.0e-5,
        ));
        let influence_impact = max(
            influence_h / influence_energy_scale,
            critical_impact * 1.08,
        );
        // Convert conserved-impact distance from the requested outer boundary to
        // approximate output pixels. The last band contains no displaced sample:
        // UVs return first, then alpha fades, preventing a compressed bright ring.
        let impact_per_pixel = 2.0 * camera_radius
            * max(params.viewport_time_fov.w, 1.0e-4)
            / max(viewport_size.y, 1.0);
        let edge_distance_pixels = max(
            (influence_impact - ray.impact_parameter)
                / max(impact_per_pixel, 1.0e-4),
            0.0,
        );
        lens_edge_coverage = smootherstep01(
            edge_distance_pixels / LENS_ALPHA_FEATHER_PIXELS,
        );
        let lens_uv_support = smootherstep01(
            (edge_distance_pixels - LENS_ALPHA_FEATHER_PIXELS)
                / LENS_UV_RETURN_PIXELS,
        );

        let critical_response = impact_response * (0.55 + 0.45 * impact_response);
        let winding_response = 1.0
            - (1.0 - closest_response) * (1.0 - path_response);
        let physical_confidence = clamp(
            0.58 * critical_response + 0.42 * winding_response,
            0.0,
            1.0,
        );
        let raw_visible_displacement = smoothstep(0.18, 2.25, raw_warp_pixels);
        let raw_perceptual_displacement = 1.0 - exp(-0.58 * raw_warp_pixels);
        let raw_displacement_response = raw_visible_displacement
            * raw_perceptual_displacement;
        let physical_support = clamp(
            raw_displacement_response * physical_confidence
                * projection.coverage * unbent_projection.coverage,
            0.0,
            1.0,
        );

        // Explorer owns the native drag thumbnail, so the renderer cannot bend
        // those pixels directly. A short, geodesically occluded tidal echo in
        // the desktop capture shows radial stretching toward the hole without
        // introducing the angular motion of the accretion disk.
        let cursor_pixels = params.drag_feedback.xy * viewport_size;
        let fragment_pixels = uv * viewport_size;
        let cursor_to_hole = viewport_size * 0.5 - cursor_pixels;
        let inward = cursor_to_hole / max(length(cursor_to_hole), 1.0);
        let tangent = vec2<f32>(-inward.y, inward.x);
        let relative = fragment_pixels - cursor_pixels;
        let along = dot(relative, inward);
        let across = dot(relative, tangent);
        let drag_influence = clamp(params.drag_feedback.z, 0.0, 1.0);
        let wake_length = mix(34.0, 148.0, drag_influence);
        let wake_width = mix(8.0, 22.0, drag_influence);
        let longitudinal_gate = smoothstep(-10.0, 4.0, along)
            * (1.0 - smoothstep(wake_length * 0.56, wake_length, along));
        let transverse_gate = exp(-pow(across / max(wake_width, 1.0), 2.0));
        let core_coordinate = vec2<f32>(
            along / max(wake_width * 1.45, 1.0),
            across / max(wake_width, 1.0),
        );
        let local_core = exp(-dot(core_coordinate, core_coordinate));
        let tidal_mask = drag_influence * clamp(
            local_core * 0.38 + longitudinal_gate * transverse_gate * 0.72,
            0.0,
            1.0,
        );
        let target_response = mix(
            0.92,
            1.08,
            clamp(params.drag_feedback.w, 0.0, 1.0),
        );
        let tidal_shift_uv = inward * tidal_mask
            * mix(0.8, 5.2, drag_influence) * target_response / viewport_size;

        // Mapping support controls only how the sampled coordinate returns to the
        // unwarped desktop at the far field and finite-capture boundary. Optical
        // replacement coverage is derived separately below: using this gradual
        // mapping value as window alpha would blend the warped capture over the
        // real desktop a second time and create a visible double image.
        let tentative_deflection = (raw_deflection_uv * warp_amount + tidal_shift_uv)
            * physical_support * lens_uv_support;
        let mapping = params.desktop_uv_origin_scale;
        let capture_size = vec2<f32>(textureDimensions(desktop_texture));
        let base_capture_uv = mapping.xy + uv * mapping.zw;
        let base_edge_pixels = min(
            min(
                base_capture_uv.x * capture_size.x,
                (1.0 - base_capture_uv.x) * capture_size.x,
            ),
            min(
                base_capture_uv.y * capture_size.y,
                (1.0 - base_capture_uv.y) * capture_size.y,
            ),
        );
        let tentative_capture_uv = mapping.xy + (uv + tentative_deflection) * mapping.zw;
        let tentative_edge_pixels = min(
            min(
                tentative_capture_uv.x * capture_size.x,
                (1.0 - tentative_capture_uv.x) * capture_size.x,
            ),
            min(
                tentative_capture_uv.y * capture_size.y,
                (1.0 - tentative_capture_uv.y) * capture_size.y,
            ),
        );
        // C1 capture support prevents large near-critical deflections from
        // turning insufficient overscan into a sharp transparent boundary.
        let capture_support = smoothstep(0.5, 16.0, base_edge_pixels)
            * smoothstep(-32.0, 16.0, tentative_edge_pixels);
        let mapping_strength = physical_support * lens_uv_support * capture_support;
        let mapped_deflection_uv = (raw_deflection_uv * warp_amount + tidal_shift_uv)
            * mapping_strength;
        let provisional_capture_uv = base_capture_uv + mapped_deflection_uv * mapping.zw;
        let capture_edge_pixels = min(
            min(
                provisional_capture_uv.x * capture_size.x,
                (1.0 - provisional_capture_uv.x) * capture_size.x,
            ),
            min(
                provisional_capture_uv.y * capture_size.y,
                (1.0 - provisional_capture_uv.y) * capture_size.y,
            ),
        );
        // When overscan is insufficient, return the sample coordinate to the
        // unwarped desktop across a broad capture-space band. Fading validity
        // directly would reveal the live desktop underneath a displaced sample.
        let capture_mapping_support = smoothstep(-8.0, 16.0, capture_edge_pixels);
        let base_mapping_support = smoothstep(-0.5, 5.5, base_edge_pixels);
        let safe_deflection_uv = mapped_deflection_uv
            * capture_mapping_support * base_mapping_support;
        let safe_capture_uv = base_capture_uv + safe_deflection_uv * mapping.zw;
        lens_desktop_color = textureSampleLevel(
            desktop_texture,
            desktop_sampler,
            clamp(safe_capture_uv, vec2<f32>(0.0), vec2<f32>(1.0)),
            0.0,
        ).rgb;
        if (max(warp_amount, drag_influence) > 1.0e-4) {
            lens_active = 1.0;
        }
    }

    let replacement_coverage = lens_edge_coverage * lens_active;
    let desktop_weight = ray.transmittance * replacement_coverage;
    premultiplied_rgb += lens_desktop_color * desktop_weight;
    alpha += desktop_weight;

    alpha = clamp(alpha, 0.0, 1.0);
    // The DComp surface consumes premultiplied RGBA. Escaping rays introduce no
    // star/sky color; without desktop capture they remain fully transparent.
    let bounded_rgb = min(
        max(premultiplied_rgb, vec3<f32>(0.0)),
        vec3<f32>(alpha),
    );
    return vec4<f32>(bounded_rgb, alpha);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<f32>(1.0) / max(
        params.viewport_time_fov.xy,
        vec2<f32>(1.0),
    );
    let minimum_uv = pixel * 0.5;
    let maximum_uv = vec2<f32>(1.0) - minimum_uv;
    let sample_uv = clamp(
        in.uv + pixel * params.sample.xy,
        minimum_uv,
        maximum_uv,
    );
    // SSAA is composed from four additive Material2d draws. Keeping each complete
    // ray march outside another shader loop avoids pathological DXC compile time,
    // while weighted premultiplied RGBA remains an exact spatial average.
    return shade_sample(sample_uv) * params.sample.z;
}
