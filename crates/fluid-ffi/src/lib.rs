//! The C ABI the iOS shell links. `include/fluid_ffi.h` is generated from
//! this file by cbindgen; the gate fails when the two drift.

use fluid_core::MotionSample;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FluidVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<FluidVec3> for [f32; 3] {
    fn from(v: FluidVec3) -> Self {
        [v.x, v.y, v.z]
    }
}

impl From<[f32; 3]> for FluidVec3 {
    fn from([x, y, z]: [f32; 3]) -> Self {
        FluidVec3 { x, y, z }
    }
}

/// Body force per unit mass, in metres per second squared, from CoreMotion's
/// `gravity` and `userAcceleration` (both in g).
#[unsafe(no_mangle)]
pub extern "C" fn fluid_body_force(gravity: FluidVec3, user_acceleration: FluidVec3) -> FluidVec3 {
    MotionSample {
        gravity: gravity.into(),
        user_acceleration: user_acceleration.into(),
    }
    .body_force()
    .into()
}
