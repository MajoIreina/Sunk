struct Parameters {
    resolution: vec2<f32>,
    elapsed_seconds: f32,
    interaction: f32,
    ray_steps: u32,
    premultiply_output: u32,
    _padding: vec2<u32>,
};

@group(0) @binding(0)
var<uniform> params: Parameters;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct DiskSample {
    radiance: vec3<f32>,
    opacity: f32,
};

const EVENT_HORIZON: f32 = 1.0;
const PHOTON_SPHERE_IMPACT: f32 = 2.598076;
const CAMERA_DISTANCE: f32 = 14.0;
const DISK_INNER_RADIUS: f32 = 1.72;
const DISK_OUTER_RADIUS: f32 = 6.8;
const MAX_RAY_STEPS: u32 = 96u;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.uv = positions[vertex_index] * 0.5 + vec2<f32>(0.5);
    return output;
}

fn gravity(position: vec3<f32>, angular_momentum_squared: f32) -> vec3<f32> {
    let radius_squared = max(dot(position, position), 0.0001);
    let inverse_radius = inverseSqrt(radius_squared);
    let inverse_radius_fifth = inverse_radius * inverse_radius * inverse_radius * inverse_radius * inverse_radius;
    return -1.5 * angular_momentum_squared * position * inverse_radius_fifth;
}

fn safe_normalize(value: vec3<f32>) -> vec3<f32> {
    return value * inverseSqrt(max(dot(value, value), 0.000001));
}

fn hash_noise(coordinates: vec2<f32>) -> f32 {
    let dot_product = dot(coordinates, vec2<f32>(127.1, 311.7));
    return fract(sin(dot_product) * 43758.5453);
}

fn value_noise(coordinates: vec2<f32>) -> f32 {
    let cell = floor(coordinates);
    let local = fract(coordinates);
    let blend = local * local * (vec2<f32>(3.0) - 2.0 * local);
    let lower = mix(hash_noise(cell), hash_noise(cell + vec2<f32>(1.0, 0.0)), blend.x);
    let upper = mix(
        hash_noise(cell + vec2<f32>(0.0, 1.0)),
        hash_noise(cell + vec2<f32>(1.0, 1.0)),
        blend.x,
    );
    return mix(lower, upper, blend.y);
}

fn thermal_color(temperature: f32) -> vec3<f32> {
    let ember = vec3<f32>(0.42, 0.012, 0.0015);
    let orange = vec3<f32>(1.0, 0.16, 0.012);
    let gold = vec3<f32>(1.0, 0.58, 0.12);
    let white_hot = vec3<f32>(1.0, 0.94, 0.74);

    let warm = mix(ember, orange, smoothstep(0.08, 0.38, temperature));
    let hot = mix(gold, white_hot, smoothstep(0.78, 1.28, temperature));
    return mix(warm, hot, smoothstep(0.32, 0.92, temperature));
}

fn sample_accretion_disk(
    hit: vec3<f32>,
    radius: f32,
    disk_normal: vec3<f32>,
    ray_direction: vec3<f32>,
    image_order: u32,
) -> DiskSample {
    let radial_direction = safe_normalize(hit - disk_normal * dot(hit, disk_normal));
    let orbital_direction = safe_normalize(cross(disk_normal, radial_direction));
    let direction_to_observer = safe_normalize(-ray_direction);

    let beta = clamp(sqrt(0.5 / max(radius, 1.05)), 0.0, 0.68);
    let gamma = inverseSqrt(max(1.0 - beta * beta, 0.05));
    let line_of_sight_velocity = dot(orbital_direction, direction_to_observer);
    let doppler = clamp(1.0 / (gamma * (1.0 - beta * line_of_sight_velocity)), 0.48, 1.72);
    let gravitational_shift = sqrt(max(1.0 - EVENT_HORIZON / radius, 0.025));
    let frequency_shift = clamp(gravitational_shift * doppler, 0.42, 1.62);

    let disk_v_axis = safe_normalize(cross(disk_normal, vec3<f32>(1.0, 0.0, 0.0)));
    let azimuth = atan2(dot(hit, disk_v_axis), hit.x);
    let orbit_rate = pow(max(radius, DISK_INNER_RADIUS), -1.5);
    let flow = vec2<f32>(
        azimuth * 1.35 - params.elapsed_seconds * (0.28 + 1.8 * orbit_rate),
        log(radius) * 5.4,
    );
    let turbulence = value_noise(flow * 1.8) * 0.68 + value_noise(flow * 3.7 + 9.2) * 0.32;
    let spiral_phase = azimuth * 5.0 + log(radius) * 15.5 - params.elapsed_seconds * (0.9 + 4.0 * orbit_rate);
    let filament = smoothstep(0.12, 0.96, 0.5 + 0.5 * sin(spiral_phase + turbulence * 3.2));
    let texture = 0.58 + 0.26 * filament + 0.16 * turbulence;

    let inner_edge = smoothstep(DISK_INNER_RADIUS, DISK_INNER_RADIUS + 0.32, radius);
    let outer_edge = 1.0 - smoothstep(DISK_OUTER_RADIUS - 1.45, DISK_OUTER_RADIUS, radius);
    let edge_mask = inner_edge * outer_edge;
    let no_torque = pow(max(1.0 - sqrt(DISK_INNER_RADIUS / radius), 0.0), 0.25);
    let radial_temperature = pow(DISK_INNER_RADIUS / radius, 0.75) * no_torque;
    let temperature = radial_temperature * frequency_shift * 3.15;
    let beaming = clamp(doppler * doppler * doppler, 0.16, 3.8);
    let inner_emission = 0.54 + 1.08 * exp(-(radius - DISK_INNER_RADIUS) * 0.72);
    let radial_flux = pow(DISK_INNER_RADIUS / radius, 1.35);
    let interaction_exposure = 1.0 + params.interaction * 0.32;
    let image_attenuation = pow(0.29, f32(image_order));

    let radiance = thermal_color(temperature)
        * edge_mask
        * texture
        * beaming
        * inner_emission
        * radial_flux
        * interaction_exposure
        * image_attenuation;
    let opacity = clamp(
        edge_mask
            * (0.22 + 0.38 * texture)
            * sqrt(radial_flux)
            * image_attenuation,
        0.0,
        0.88,
    );
    return DiskSample(radiance, opacity);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = params.resolution.x / max(params.resolution.y, 1.0);
    var screen_position = input.uv * 2.0 - vec2<f32>(1.0);
    screen_position.x *= aspect;

    let world_scale = PHOTON_SPHERE_IMPACT / 0.225;
    let impact = screen_position * world_scale;
    let impact_radius = length(impact);
    let ring_pixel_width = max(fwidth(impact_radius), 0.012);

    if abs(screen_position.x) > 0.88 || abs(screen_position.y) > 0.61 {
        return vec4<f32>(0.0);
    }

    let disk_inclination = radians(82.0);
    let disk_normal = vec3<f32>(0.0, sin(disk_inclination), cos(disk_inclination));
    var position = vec3<f32>(impact, CAMERA_DISTANCE);
    var direction = vec3<f32>(0.0, 0.0, -1.0);
    let angular_momentum = cross(position, direction);
    let angular_momentum_squared = dot(angular_momentum, angular_momentum);
    let step_budget = clamp(params.ray_steps, 64u, MAX_RAY_STEPS);

    var accumulated_radiance = vec3<f32>(0.0);
    var transmittance = 1.0;
    var captured = false;
    var crossed_disk = 0u;

    for (var step = 0u; step < MAX_RAY_STEPS; step += 1u) {
        if step >= step_budget {
            break;
        }

        let radius = length(position);
        if radius <= EVENT_HORIZON {
            captured = true;
            break;
        }
        if radius > CAMERA_DISTANCE + 0.8 && dot(position, direction) > 0.0 {
            break;
        }

        let previous_position = position;
        let previous_plane_distance = dot(previous_position, disk_normal);
        let time_step = clamp(radius * 0.09, 0.018, 0.5);

        direction += gravity(position, angular_momentum_squared) * (0.5 * time_step);
        position += direction * time_step;
        direction += gravity(position, angular_momentum_squared) * (0.5 * time_step);

        let current_plane_distance = dot(position, disk_normal);
        if previous_plane_distance * current_plane_distance <= 0.0 && crossed_disk < 4u {
            let denominator = previous_plane_distance - current_plane_distance;
            let crossing_fraction = clamp(previous_plane_distance / (denominator + select(-0.000001, 0.000001, denominator >= 0.0)), 0.0, 1.0);
            let hit = mix(previous_position, position, crossing_fraction);
            let disk_position = hit - disk_normal * dot(hit, disk_normal);
            let disk_radius = length(disk_position);

            if disk_radius >= DISK_INNER_RADIUS && disk_radius <= DISK_OUTER_RADIUS {
                let sample = sample_accretion_disk(
                    disk_position,
                    disk_radius,
                    disk_normal,
                    direction,
                    crossed_disk,
                );
                accumulated_radiance += transmittance * sample.radiance * sample.opacity;
                transmittance *= 1.0 - sample.opacity;
                crossed_disk += 1u;
            }
        }
    }

    let distance_to_critical = abs(impact_radius - PHOTON_SPHERE_IMPACT);
    let photon_core = 1.0 - smoothstep(ring_pixel_width * 0.22, ring_pixel_width * 0.82 + 0.01, distance_to_critical);
    let photon_halo = exp(-distance_to_critical * 18.0) * 0.07;
    let photon_strength = max(photon_core, photon_halo) * (0.82 + params.interaction * 0.24);
    let ring_opacity = photon_core * 0.34 + photon_halo * 0.08;
    let ring_radiance = vec3<f32>(1.0, 0.72, 0.34) * photon_strength * 3.1;
    accumulated_radiance += transmittance * ring_radiance * ring_opacity;
    transmittance *= 1.0 - ring_opacity;

    let shadow_ray = captured || impact_radius < PHOTON_SPHERE_IMPACT;
    let emitted_alpha = 1.0 - transmittance;
    let alpha = select(emitted_alpha, 1.0, shadow_ray);
    if alpha <= 0.0001 {
        return vec4<f32>(0.0);
    }

    let straight_radiance = select(
        accumulated_radiance / max(emitted_alpha, 0.0001),
        accumulated_radiance,
        shadow_ray,
    );
    let exposure = 1.18 + params.interaction * 0.16;
    let tone_mapped = vec3<f32>(1.0) - exp(-straight_radiance * exposure);
    let output_color = select(tone_mapped, tone_mapped * alpha, params.premultiply_output != 0u);
    return vec4<f32>(output_color, alpha);
}
