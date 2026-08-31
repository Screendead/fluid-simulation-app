//! The platform-free core. Every sensor reading enters as a [`MotionSample`];
//! no platform type crosses this boundary.

/// Standard gravity in metres per second squared.
pub const STANDARD_GRAVITY: f32 = 9.806_65;

/// The modeled tank is this many times the physical device in every
/// dimension; the sensors still feed real m/s^2, so the screen is a
/// window into a larger body of water. Jack's pick, 2026-08-31, from the
/// measured scale ladder (M3 record, "The scale, chosen"). Every length
/// that describes the modeled world carries this factor; a length that
/// describes the device or its sensors must not.
pub const WORLD_SCALE: f32 = 4.0;

/// One reading of the motion sensors in the device frame: x to the right of
/// the screen, y to its top, z out of it. Both vectors are in g, the CoreMotion
/// convention: a phone lying face up reads a gravity of (0, 0, -1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionSample {
    pub gravity: [f32; 3],
    pub user_acceleration: [f32; 3],
}

impl MotionSample {
    /// Body force per unit mass on fluid in a box fixed to the device, in
    /// metres per second squared, device frame. This is the negated proper
    /// acceleration: pushing the phone right throws the fluid left.
    pub fn body_force(&self) -> [f32; 3] {
        std::array::from_fn(|i| STANDARD_GRAVITY * (self.gravity[i] - self.user_acceleration[i]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_rest_face_up_the_fluid_falls_into_the_screen() {
        let sample = MotionSample {
            gravity: [0.0, 0.0, -1.0],
            user_acceleration: [0.0; 3],
        };
        assert_eq!(sample.body_force(), [0.0, 0.0, -STANDARD_GRAVITY]);
    }

    #[test]
    fn pushing_right_throws_the_fluid_left() {
        let sample = MotionSample {
            gravity: [0.0, 0.0, -1.0],
            user_acceleration: [0.5, 0.0, 0.0],
        };
        let [x, _, _] = sample.body_force();
        assert_eq!(x, -0.5 * STANDARD_GRAVITY);
    }
}

mod particles;
mod render;
mod sim;
#[cfg(feature = "film")]
pub use render::film;
pub use render::{RenderOptions, RenderStats, Renderer};
