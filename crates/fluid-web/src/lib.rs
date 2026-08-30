//! The wasm-bindgen surface the web page calls.

use fluid_core::{MotionSample, STANDARD_GRAVITY};
use wasm_bindgen::prelude::*;

/// Converts the two `DeviceMotionEvent` vectors, in metres per second
/// squared, to a sample. `accelerationIncludingGravity` is the reaction to
/// gravity, so a phone face up reads z = +9.8: the opposite sign to CoreMotion.
pub fn sample_from_device_motion(including_gravity: [f32; 3], acceleration: [f32; 3]) -> MotionSample {
    MotionSample {
        gravity: std::array::from_fn(|i| (acceleration[i] - including_gravity[i]) / STANDARD_GRAVITY),
        user_acceleration: acceleration.map(|a| a / STANDARD_GRAVITY),
    }
}

/// Body force per unit mass in metres per second squared, device frame, from
/// `DeviceMotionEvent.accelerationIncludingGravity` and `.acceleration`.
#[wasm_bindgen]
pub fn body_force(gx: f32, gy: f32, gz: f32, ax: f32, ay: f32, az: f32) -> Vec<f32> {
    sample_from_device_motion([gx, gy, gz], [ax, ay, az]).body_force().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_up_at_rest_matches_the_coremotion_convention() {
        let sample = sample_from_device_motion([0.0, 0.0, STANDARD_GRAVITY], [0.0; 3]);
        assert_eq!(sample.gravity, [0.0, 0.0, -1.0]);
        assert_eq!(sample.user_acceleration, [0.0; 3]);
    }

    #[test]
    fn body_force_is_the_negated_accelerometer_reading() {
        let including_gravity = [1.0, -2.0, 9.0];
        let [x, y, z] = sample_from_device_motion(including_gravity, [0.3, 0.1, -0.4]).body_force();
        assert!((x + 1.0).abs() < 1e-5 && (y - 2.0).abs() < 1e-5 && (z + 9.0).abs() < 1e-5);
    }
}
