//! CPU regression oracle for the Schwarzschild ray model used by the WGSL shader.
//!
//! Spatial distances are normalized by the Schwarzschild radius (`r_s = 1`). The
//! production renderer traces the same central-force form with adaptive midpoint
//! or RK4 integration; this f64 implementation provides stable reference values.

use bevy::math::DVec3;

pub const EVENT_HORIZON_RS: f32 = 1.0;
pub const PHOTON_SPHERE_RS: f64 = 1.5;
pub const MARGINALLY_BOUND_ORBIT_RS: f64 = 2.0;
pub const ISCO_RS: f32 = 3.0;
pub const CRITICAL_IMPACT_PARAMETER_RS: f64 = 2.598_076_211_353_316;

pub const GRAVITATIONAL_CONSTANT: f64 = 6.674_30e-11;
pub const SPEED_OF_LIGHT: f64 = 299_792_458.0;

const REFERENCE_OBSERVER_RADIUS: f64 = 60.0;
const REFERENCE_ESCAPE_RADIUS: f64 = 80.0;
const TURN_TOLERANCE_RADIANS: f64 = 0.04;
const MIN_STEP: f64 = 0.002;
const MAX_STEP: f64 = 2.5;

pub fn schwarzschild_radius(mass_kg: f64) -> f64 {
    2.0 * GRAVITATIONAL_CONSTANT * mass_kg / SPEED_OF_LIGHT.powi(2)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RayOutcome {
    Captured,
    Escaped,
    Unresolved,
}

/// Diagnostic result exposed so integration changes can be checked without
/// duplicating the reference tracer in benchmarks or future GPU parity tests.
#[derive(Debug, Clone, Copy)]
pub struct ReferenceTrace {
    pub outcome: RayOutcome,
    pub steps: u32,
    pub effective_impact_parameter: f64,
    pub max_energy_drift: f64,
    pub max_angular_momentum_relative_drift: f64,
}

#[derive(Debug, Clone, Copy)]
struct RayState {
    position: DVec3,
    velocity: DVec3,
}

fn schwarzschild_acceleration(position: DVec3, h_squared: f64) -> DVec3 {
    let radius_squared = position.length_squared().max(1.0e-16);
    let inverse_radius = radius_squared.sqrt().recip();
    let inverse_radius_squared = radius_squared.recip();
    let inverse_radius_fifth = inverse_radius * inverse_radius_squared * inverse_radius_squared;
    -1.5 * h_squared * position * inverse_radius_fifth
}

fn ray_derivative(state: RayState, h_squared: f64) -> RayState {
    RayState {
        position: state.velocity,
        velocity: schwarzschild_acceleration(state.position, h_squared),
    }
}

fn state_offset(state: RayState, derivative: RayState, scale: f64) -> RayState {
    RayState {
        position: state.position + derivative.position * scale,
        velocity: state.velocity + derivative.velocity * scale,
    }
}

fn rk4_step(state: RayState, h_squared: f64, step_size: f64) -> RayState {
    let k1 = ray_derivative(state, h_squared);
    let k2 = ray_derivative(state_offset(state, k1, 0.5 * step_size), h_squared);
    let k3 = ray_derivative(state_offset(state, k2, 0.5 * step_size), h_squared);
    let k4 = ray_derivative(state_offset(state, k3, step_size), h_squared);

    RayState {
        position: state.position
            + (k1.position + 2.0 * k2.position + 2.0 * k3.position + k4.position)
                * (step_size / 6.0),
        velocity: state.velocity
            + (k1.velocity + 2.0 * k2.velocity + 2.0 * k3.velocity + k4.velocity)
                * (step_size / 6.0),
    }
}

fn orbit_energy(state: RayState, h_squared: f64) -> f64 {
    let radius = state.position.length().max(1.0e-8);
    0.5 * state.velocity.length_squared() - 0.5 * h_squared / radius.powi(3)
}

fn adaptive_step_size(state: RayState, h_squared: f64) -> f64 {
    let radius = state
        .position
        .length()
        .max(f64::from(EVENT_HORIZON_RS) + 1.0e-8);
    let speed_squared = state.velocity.length_squared().max(1.0e-16);
    let speed = speed_squared.sqrt();
    let acceleration = schwarzschild_acceleration(state.position, h_squared);
    let perpendicular_acceleration =
        acceleration - state.velocity * acceleration.dot(state.velocity) / speed_squared;

    let turn_step =
        TURN_TOLERANCE_RADIANS * speed / perpendicular_acceleration.length().max(1.0e-10);
    let geometry_step = 0.12 * radius / speed;
    let radial_direction = state.position / radius;
    let radial_speed = state.velocity.dot(radial_direction);
    let horizon_step = if radial_speed < 0.0 {
        0.25 * (radius - f64::from(EVENT_HORIZON_RS)).max(0.001) / (-radial_speed).max(0.05)
    } else {
        MAX_STEP
    };

    turn_step
        .min(geometry_step)
        .min(horizon_step)
        .clamp(MIN_STEP, MAX_STEP)
}

fn segment_sphere_first_hit(start: DVec3, end: DVec3, radius: f64) -> Option<f64> {
    let delta = end - start;
    let quadratic_a = delta.length_squared();
    if quadratic_a <= 1.0e-16 {
        return None;
    }

    let quadratic_b = start.dot(delta);
    let quadratic_c = start.length_squared() - radius * radius;
    let discriminant = quadratic_b * quadratic_b - quadratic_a * quadratic_c;
    if discriminant < 0.0 {
        return None;
    }

    let root = discriminant.max(0.0).sqrt();
    let near_fraction = (-quadratic_b - root) / quadratic_a;
    if (0.0..=1.0).contains(&near_fraction) {
        return Some(near_fraction);
    }
    let far_fraction = (-quadratic_b + root) / quadratic_a;
    (0.0..=1.0).contains(&far_fraction).then_some(far_fraction)
}

/// Trace an asymptotically incoming ray using the same adaptive RK4 policy as
/// the shader's high-quality mode.
///
/// The observer is finite, so the supplied screen-space offset is converted to
/// an effective asymptotic impact parameter using the conserved orbit energy.
pub fn trace_reference_ray_diagnostics(impact_parameter: f64, max_steps: u32) -> ReferenceTrace {
    let mut state = RayState {
        position: DVec3::new(impact_parameter, 0.0, REFERENCE_OBSERVER_RADIUS),
        velocity: DVec3::new(0.0, 0.0, -1.0),
    };
    let initial_angular_momentum = state.position.cross(state.velocity);
    let h_squared = initial_angular_momentum.length_squared();
    let initial_energy = orbit_energy(state, h_squared);
    let effective_impact_parameter = h_squared.sqrt() / (2.0 * initial_energy).max(1.0e-16).sqrt();
    let angular_momentum_scale = initial_angular_momentum.length().max(1.0);
    let mut max_energy_drift: f64 = 0.0;
    let mut max_angular_momentum_relative_drift: f64 = 0.0;

    for index in 0..max_steps {
        let radius = state.position.length();
        if radius <= f64::from(EVENT_HORIZON_RS) {
            return ReferenceTrace {
                outcome: RayOutcome::Captured,
                steps: index,
                effective_impact_parameter,
                max_energy_drift,
                max_angular_momentum_relative_drift,
            };
        }
        if index > 4 && radius > REFERENCE_ESCAPE_RADIUS && state.position.dot(state.velocity) > 0.0
        {
            return ReferenceTrace {
                outcome: RayOutcome::Escaped,
                steps: index,
                effective_impact_parameter,
                max_energy_drift,
                max_angular_momentum_relative_drift,
            };
        }

        let step_size = adaptive_step_size(state, h_squared);
        let next_state = rk4_step(state, h_squared, step_size);
        let horizon_hit = segment_sphere_first_hit(
            state.position,
            next_state.position,
            f64::from(EVENT_HORIZON_RS),
        );
        state = next_state;

        let energy_drift = (orbit_energy(state, h_squared) - initial_energy).abs();
        max_energy_drift = max_energy_drift.max(energy_drift);
        let angular_momentum_drift =
            (state.position.cross(state.velocity) - initial_angular_momentum).length()
                / angular_momentum_scale;
        max_angular_momentum_relative_drift =
            max_angular_momentum_relative_drift.max(angular_momentum_drift);

        if horizon_hit.is_some() {
            return ReferenceTrace {
                outcome: RayOutcome::Captured,
                steps: index + 1,
                effective_impact_parameter,
                max_energy_drift,
                max_angular_momentum_relative_drift,
            };
        }
    }

    ReferenceTrace {
        outcome: RayOutcome::Unresolved,
        steps: max_steps,
        effective_impact_parameter,
        max_energy_drift,
        max_angular_momentum_relative_drift,
    }
}

pub fn trace_reference_ray(impact_parameter: f64, max_steps: u32) -> RayOutcome {
    trace_reference_ray_diagnostics(impact_parameter, max_steps).outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characteristic_schwarzschild_radii_are_consistent() {
        assert_eq!(PHOTON_SPHERE_RS, 1.5);
        assert_eq!(MARGINALLY_BOUND_ORBIT_RS, 2.0);
        assert_eq!(ISCO_RS, 3.0);
        assert!((CRITICAL_IMPACT_PARAMETER_RS - 3.0 * 3.0_f64.sqrt() / 2.0).abs() < 1.0e-14);
    }

    #[test]
    fn reference_mass_has_expected_schwarzschild_scale() {
        const SOLAR_MASS_KG: f64 = 1.988_47e30;
        let radius = schwarzschild_radius(14_900_000.0 * SOLAR_MASS_KG);
        assert!((radius - 4.400e10).abs() / radius < 0.002);
    }

    #[test]
    fn critical_impact_parameter_separates_capture_and_escape() {
        let captured = trace_reference_ray(CRITICAL_IMPACT_PARAMETER_RS * 0.98, 2_000);
        let escaped = trace_reference_ray(CRITICAL_IMPACT_PARAMETER_RS * 1.02, 2_000);
        assert_eq!(captured, RayOutcome::Captured);
        assert_eq!(escaped, RayOutcome::Escaped);
    }

    #[test]
    fn near_critical_shadow_edge_does_not_drift() {
        let captured = trace_reference_ray(2.59, 2_000);
        let escaped = trace_reference_ray(2.61, 2_000);
        assert_eq!(captured, RayOutcome::Captured);
        assert_eq!(escaped, RayOutcome::Escaped);
    }

    #[test]
    fn radial_ray_is_captured() {
        assert_eq!(trace_reference_ray(0.0, 1_000), RayOutcome::Captured);
    }

    #[test]
    fn adaptive_rk4_preserves_orbit_invariants() {
        let trace = trace_reference_ray_diagnostics(3.0, 1_000);
        assert_eq!(trace.outcome, RayOutcome::Escaped);
        assert!(
            trace.max_energy_drift < 1.0e-5,
            "energy drift was {}",
            trace.max_energy_drift
        );
        assert!(
            trace.max_angular_momentum_relative_drift < 1.0e-5,
            "angular momentum drift was {}",
            trace.max_angular_momentum_relative_drift
        );
    }

    #[test]
    fn finite_observer_offset_is_converted_to_asymptotic_impact() {
        let trace = trace_reference_ray_diagnostics(4.0, 1_000);
        assert!(trace.effective_impact_parameter > 4.0);
        assert!(trace.effective_impact_parameter - 4.0 < 0.01);
    }

    #[test]
    fn adaptive_far_field_step_is_not_capped_at_legacy_size() {
        let state = RayState {
            position: DVec3::new(4.0, 0.0, REFERENCE_OBSERVER_RADIUS),
            velocity: DVec3::new(0.0, 0.0, -1.0),
        };
        let h_squared = state.position.cross(state.velocity).length_squared();
        assert!(adaptive_step_size(state, h_squared) > 0.34);
    }

    #[test]
    fn segment_horizon_test_catches_a_complete_tunnel() {
        let hit =
            segment_sphere_first_hit(DVec3::new(0.0, 0.0, 2.0), DVec3::new(0.0, 0.0, -2.0), 1.0);
        assert!((hit.expect("segment should cross the horizon") - 0.25).abs() < 1.0e-12);
    }
}
