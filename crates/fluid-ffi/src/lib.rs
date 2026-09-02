//! The C ABI the iOS shell links. `include/fluid_ffi.h` is generated from
//! this file by cbindgen; the gate fails when the two drift.

use fluid_core::{Lens, Look, MotionSample, Paint};

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

use std::ffi::c_void;

/// The renderer behind the C ABI: an opaque box the Swift shell drives by
/// pointer, on the main thread only.
pub struct FluidRenderer(fluid_core::Renderer);

#[repr(C)]
pub struct FluidRenderStats {
    pub frames: u64,
    pub interval_p50_us: f32,
    pub interval_p99_us: f32,
    pub interval_max_us: f32,
    pub acquire_p50_us: f32,
    pub acquire_p99_us: f32,
    pub encode_p50_us: f32,
    pub encode_p99_us: f32,
    pub gpu_p50_us: f32,
    pub gpu_p99_us: f32,
    pub compression_avg: f32,
    pub compression_max: f32,
    pub density_min: f32,
    pub density_max: f32,
    pub pressure_min: f32,
    pub pressure_max: f32,
    pub v_max: f32,
    pub temperature_min: f32,
    pub temperature_max: f32,
    pub clamp_count: u32,
    pub neighbour_overflow: u32,
    pub substeps: u32,
    pub idle_frames: u64,
}

/// Builds the renderer on a layer: `particle_count` sprites of
/// `sprite_radius` metres, or, when `bench_sweeps` is nonzero, the M3
/// stage-0 microbench at `bench_spacing` metres. A nonzero `sim_substeps`
/// runs the M3 fluid instead, at that many substeps a frame, with
/// `particle_scale` times the shipped particle count unless
/// `bench_spacing` pins the spacing. Returns null when GPU setup fails,
/// with the reason on stderr.
///
/// # Safety
///
/// `metal_layer` must be a `CAMetalLayer` pointer, kept alive until
/// `fluid_renderer_destroy`. Call on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_create(
    metal_layer: *mut c_void,
    width: u32,
    height: u32,
    particle_count: u32,
    sprite_radius: f32,
    bench_sweeps: u32,
    bench_spacing: f32,
    sim_substeps: u32,
    tracers: u32,
    particle_scale: f32,
) -> *mut FluidRenderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let target = wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(metal_layer);
    let surface = match unsafe { instance.create_surface_unsafe(target) } {
        Ok(surface) => surface,
        Err(e) => {
            eprintln!("fluid: no surface on the layer: {e}");
            return std::ptr::null_mut();
        }
    };
    let options = fluid_core::RenderOptions {
        particle_count,
        sprite_radius,
        bench_sweeps,
        bench_spacing,
        sim_substeps,
        tracers,
        particle_scale,
    };
    match fluid_core::Renderer::new(instance, surface, width, height, options) {
        Ok(renderer) => Box::into_raw(Box::new(FluidRenderer(renderer))),
        Err(e) => {
            eprintln!("fluid: no renderer: {e}");
            std::ptr::null_mut()
        }
    }
}

/// One frame: integrate the particles, draw them over the body-force tint,
/// present. `now_ms` is `CADisplayLink.timestamp` in milliseconds. Returns
/// 1 when a frame was stepped and presented, 0 when the settled sim slept
/// it; at 0 the shell may drop its tick rate until the next 1.
///
/// # Safety
///
/// `renderer` must be a live pointer from `fluid_renderer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_frame(
    renderer: *mut FluidRenderer,
    gravity: FluidVec3,
    user_acceleration: FluidVec3,
    rotation_rate: FluidVec3,
    now_ms: f64,
) -> u32 {
    let sample = MotionSample {
        gravity: gravity.into(),
        user_acceleration: user_acceleration.into(),
        rotation_rate: rotation_rate.into(),
    };
    u32::from(unsafe { &mut *renderer }.0.frame(sample, now_ms))
}

/// Reseeds the fluid at `scale` times the shipped particle count: a
/// rebuild of the sim, off the frame path, after which a still phone's
/// water falls and settles again.
///
/// # Safety
///
/// `renderer` must be a live pointer from `fluid_renderer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_set_particles(renderer: *mut FluidRenderer, scale: f32) {
    unsafe { &mut *renderer }.0.set_particles(scale);
}

/// The particle count `fluid_renderer_set_particles` would seed at
/// `scale`, for the menu's labels.
///
/// # Safety
///
/// `renderer` must be a live pointer from `fluid_renderer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_particles_at(
    renderer: *const FluidRenderer,
    scale: f32,
) -> u32 {
    unsafe { &*renderer }.0.particles_at(scale)
}

/// How many fingers the sim drags with at once. A shell must give each
/// finger a slot below this and hold it for as long as that finger
/// stays on the glass.
#[unsafe(no_mangle)]
pub static FLUID_TOUCH_SLOTS: u32 = fluid_core::TOUCH_SLOTS;

/// Where one finger presses, normalised over the drawable: `x` runs 0
/// to 1 left to right, `y` 0 to 1 top to bottom. Every finger down
/// drags the water it moves through and keeps the sim awake; `down`
/// false lifts one. Call it from the gesture, not the frame.
///
/// # Safety
///
/// `renderer` must be a live pointer from `fluid_renderer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_touch(
    renderer: *mut FluidRenderer,
    slot: u32,
    x: f32,
    y: f32,
    down: bool,
) {
    unsafe { &mut *renderer }.0.touch(slot, x, y, down);
}

/// Liquid glass when `flat` is false. Otherwise paint on black: the
/// flat surface, or, when `particles` is also true, the particles
/// alone as discs. `particles` alone does nothing.
///
/// `low` is the one colour when `gradient` is false. When it is true,
/// the colour runs from `low` to `high` across `lens`, whose range the
/// core derives from the box: 0 velocity, 1 acceleration, 2 pressure,
/// 3 proximity, 4 temperature, and anything else velocity. Colour
/// components are 0 to 1 as the picker shows them; the core
/// linearises them.
///
/// # Safety
///
/// `renderer` must be a live pointer from `fluid_renderer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_set_look(
    renderer: *mut FluidRenderer,
    flat: bool,
    particles: bool,
    low: FluidVec3,
    gradient: bool,
    high: FluidVec3,
    lens: u32,
) {
    let paint = if gradient {
        Paint::Ramp {
            low: low.into(),
            high: high.into(),
            lens: match lens {
                1 => Lens::Acceleration,
                2 => Lens::Pressure,
                3 => Lens::Proximity,
                4 => Lens::Temperature,
                _ => Lens::Velocity,
            },
        }
    } else {
        Paint::Solid(low.into())
    };
    let look = match (flat, particles) {
        (false, _) => Look::Glass,
        (true, false) => Look::Flat(paint),
        (true, true) => Look::Particles(paint),
    };
    unsafe { &mut *renderer }.0.set_look(look);
}

/// # Safety
///
/// `renderer` must be a live pointer from `fluid_renderer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_resize(
    renderer: *mut FluidRenderer,
    width: u32,
    height: u32,
) {
    unsafe { &mut *renderer }.0.resize(width, height);
}

/// # Safety
///
/// `renderer` must be a live pointer from `fluid_renderer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_stats(renderer: *const FluidRenderer) -> FluidRenderStats {
    let stats = unsafe { &*renderer }.0.stats();
    FluidRenderStats {
        frames: stats.frames,
        interval_p50_us: stats.interval_p50_us,
        interval_p99_us: stats.interval_p99_us,
        interval_max_us: stats.interval_max_us,
        acquire_p50_us: stats.acquire_p50_us,
        acquire_p99_us: stats.acquire_p99_us,
        encode_p50_us: stats.encode_p50_us,
        encode_p99_us: stats.encode_p99_us,
        gpu_p50_us: stats.gpu_p50_us,
        gpu_p99_us: stats.gpu_p99_us,
        compression_avg: stats.compression_avg,
        compression_max: stats.compression_max,
        density_min: stats.density_min,
        density_max: stats.density_max,
        pressure_min: stats.pressure_min,
        pressure_max: stats.pressure_max,
        v_max: stats.v_max,
        temperature_min: stats.temperature_min,
        temperature_max: stats.temperature_max,
        clamp_count: stats.clamp_count,
        neighbour_overflow: stats.neighbour_overflow,
        substeps: stats.substeps,
        idle_frames: stats.idle_frames,
    }
}

/// # Safety
///
/// `renderer` must come from `fluid_renderer_create`; it is dead afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluid_renderer_destroy(renderer: *mut FluidRenderer) {
    drop(unsafe { Box::from_raw(renderer) });
}
