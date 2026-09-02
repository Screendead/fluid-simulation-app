//! The particle pass: integrate under the body force in compute, draw the
//! sprites over the body-force tint, present at display rate. Frame timing
//! lands in fixed rings.

use crate::MotionSample;
use crate::particles;
use crate::sim;
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::{Context, Poll, Waker};

const RING: usize = 240;

/// The reference device's 458 ppi (CLAUDE.md section 5) times the world
/// scale; a second device would bring its own density in through the
/// shell.
const METRES_PER_PIXEL: f32 = crate::WORLD_SCALE * 0.0254 / 458.0;

/// One integration step never exceeds two 60 Hz frames, so a resume after
/// a pause cannot fling the particles.
const MAX_DT: f32 = 1.0 / 30.0;

// The resting boil is timestep error: the 2026-08-31 convergence
// ladder (M3 record) halves upright boil twice over by halving the
// substep (0.42 mm at 4.2 ms, 0.14 at 2.1) while refine depth changes
// nothing at either length. This and refine_passes key on substep
// length, not count. 2.2 ms was set 2026-08-31 against resting boil,
// before the 4x scale landed; re-measured at 4x on 2026-09-01
// (optimisation record, target 1): 4.2 ms holds every rest, ring,
// wake, flicker and shake guard, rests 120 Hz at two substeps, and
// halves the 60 Hz floor that fed the basin. Slightly above
// 8.334/2 ms, so measured 120 Hz interval jitter stays at two.
// Mirrors LANES in sim_solve.wgsl: the lane-parallel sweeps run
// SWEEP_LANES threads per particle, and a mismatch solves only a
// fraction of the fluid with no validation error to say so.
const SWEEP_LANES: u32 = 8;

/// Mirrors NBR_CAP in sim_solve.wgsl: neighbour-list slots per particle.
/// A mismatch sizes the list buffer wrong with no validation error.
const NBR_CAP: u32 = 96;

const DT_SUB_MAX: f32 = 0.0042;

// The substep floor divides the measured frame — a dropped frame
// integrated at four substeps means 4-8 ms substeps, and the device
// showed those as compression pops (compr max 6.5%, 2026-08-31). The
// cap at eight breaks the feedback that railed an uncapped floor at
// sixteen substeps and 30 Hz: past a doubled frame, more substeps
// would slow the next frame more than they converge this one.
fn substep_floor(dt: f32) -> u32 {
    ((dt / DT_SUB_MAX).ceil() as u32).min(8)
}

// Density error scales with dt squared, so short substeps need fewer
// refine passes; the film compr guard covers the shallow end. There
// is no deep branch for long substeps: the convergence ladder showed
// iterations cannot fix a 4 ms substep (the error is the timestep),
// and at 60 Hz the extra dispatches were half the solver's cost.
fn refine_passes(dt_sub: f32) -> u32 {
    if dt_sub > 0.00105 { 5 } else { 2 }
}

struct Ring {
    samples: [f32; RING],
    filled: usize,
    next: usize,
}

impl Ring {
    const fn new() -> Self {
        Ring {
            samples: [0.0; RING],
            filled: 0,
            next: 0,
        }
    }

    fn push(&mut self, v: f32) {
        self.samples[self.next] = v;
        self.next = (self.next + 1) % RING;
        self.filled = (self.filled + 1).min(RING);
    }

    /// `p` in [0, 1]. Zero while the ring is empty.
    fn percentile(&self, p: f32) -> f32 {
        if self.filled == 0 {
            return 0.0;
        }
        let mut sorted = self.samples;
        let filled = &mut sorted[..self.filled];
        filled.sort_unstable_by(f32::total_cmp);
        filled[(p * (self.filled - 1) as f32).round() as usize]
    }

    fn max(&self) -> f32 {
        self.samples[..self.filled]
            .iter()
            .copied()
            .fold(0.0, f32::max)
    }
}

/// The empty box: near-black with a cold cast, so the water alone
/// carries the scene.
const BACKDROP: wgpu::Color = wgpu::Color {
    r: 0.004,
    g: 0.008,
    b: 0.016,
    a: 1.0,
};

/// wgpu's native adapter and device futures are ready on the first poll
/// (verified in the wgpu 30.0.1 source); Pending means that contract
/// changed.
fn ready<T>(fut: impl Future<Output = T>) -> T {
    match pin!(fut).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => unreachable!("wgpu future was not ready"),
    }
}

/// None on an adapterless runner (bare CI), which callers skip.
#[cfg(any(test, feature = "film"))]
fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let Ok(adapter) = ready(instance.request_adapter(&Default::default())) else {
        eprintln!("no GPU adapter; skipping");
        return None;
    };
    let (device, queue) = ready(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: wgpu::Features::IMMEDIATES | wgpu::Features::SUBGROUP,
        required_limits: wgpu::Limits {
            max_storage_buffers_per_shader_stage: 21,
            max_immediate_size: sim::STEP_BYTES as u32,
            ..wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
        },
        ..Default::default()
    }))
    .expect("device");
    Some((device, queue))
}

/// Runs the solver against scripted input and hands every `every`-th
/// frame to `sink` as tightly packed RGBA rows. `input_at` gives the
/// frame's body force, its angular velocity, and where a finger
/// presses in normalised drawable coordinates, if one does. The box is
/// the reference phone's whatever the render size, and frames advance
/// at a fixed 1/120 s: film is the look oracle, never the cost oracle.
#[cfg(feature = "film")]
pub fn film(
    frames: u32,
    every: u32,
    spacing: f32,
    cap: u32,
    input_at: impl Fn(u32) -> ([f32; 3], [f32; 3], Option<[f32; 2]>),
    mut sink: impl FnMut(&[u8]),
) -> Option<[u32; 2]> {
    const WIDTH: u32 = 642;
    const HEIGHT: u32 = 1388;
    let (device, queue) = headless_device()?;
    let sim = Sim::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        [
            1284.0 * 0.5 * METRES_PER_PIXEL,
            2778.0 * 0.5 * METRES_PER_PIXEL,
        ],
        cap,
        spacing,
        131_072,
        [WIDTH, HEIGHT],
    );
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("film target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let padded = (WIDTH * 4).next_multiple_of(256);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("film readback"),
        size: u64::from(padded) * u64::from(HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let stats = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("film stats"),
        size: STATS_BYTES,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let dt = 1.0 / 120.0;
    let mut v_max = 0.0f32;
    let mut field_keep = FIELD_KEEP;
    // KEEP pins the field average for metrology: the boil meter must
    // see the raw field (KEEP=0), or solver churn hides behind the
    // rendered average and the metric drifts from what the eye sees.
    let keep_pin: Option<f32> = std::env::var("KEEP").ok().and_then(|v| v.parse().ok());
    let mut compr_max = 0.0f32;
    let mut clamp_total = 0.0f32;
    let mut filter = ForceFilter::new();
    let mut rotation = RotationTracker::new();
    let mut fingers = Fingers::new();
    // IDLE=0 keeps the solver stepping for metrology films: a boil or
    // jump measurement over a settled window is meaningless if the gate
    // froze the window.
    let gate_on = std::env::var("IDLE").ok().is_none_or(|v| v != "0");
    let mut gate = IdleGate::new();
    let mut idle = 0u32;
    let mut was_asleep = false;
    let mut rows = vec![0u8; (WIDTH * 4 * HEIGHT) as usize];
    for f in 0..frames {
        let (raw_force, raw_omega, at) = input_at(f);
        let (force, dev) = filter.apply(raw_force);
        let (omega, domega) = rotation.apply(raw_omega, dt);
        fingers.at[0] = at;
        let touches = fingers.step(sim.extent, dt);
        if gate_on && gate.asleep(fingers.any_down(), force, dev, omega, v_max) {
            if !was_asleep {
                eprintln!("film: sleep at frame {f}");
                was_asleep = true;
            }
            idle += 1;
            // The screen keeps the last drawable in production; the
            // film keeps the last sampled rows.
            if f % every == 0 {
                sink(&rows);
            }
            continue;
        }
        if was_asleep {
            eprintln!("film: wake at frame {f}");
            was_asleep = false;
        }
        // The production CFL and substep cap, fed by the previous
        // frame's v_max. NMIN is a diagnostic floor above the cap:
        // shortening the rest timestep separates timestep-stability
        // noise from model noise.
        let n_min: u32 = std::env::var("NMIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let n = ((dt * v_max / (0.4 * spacing)).ceil() as u32)
            .max(substep_floor(dt))
            .max(n_min)
            .min(cap);
        field_keep = match keep_pin {
            Some(k) => k,
            None => field_keep + (field_keep_target(v_max) - field_keep) * 0.25,
        };
        let dt_sub = dt / n as f32;
        let v_clamp = 0.4 * spacing / dt_sub;
        let step = sim::pack_step(force, omega, domega, dt_sub, v_clamp, 0, touches);
        let particles = sim.count.div_ceil(256);
        let wide = (sim.count * SWEEP_LANES).div_ceil(256);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            for _ in 0..n {
                pass.set_bind_group(0, &sim.grid_bind, &[]);
                pass.set_pipeline(&sim.count_cells);
                pass.dispatch_workgroups(particles, 1, 1);
                pass.set_bind_group(0, &sim.scan_bind, &[]);
                pass.set_pipeline(&sim.scan_single);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_bind_group(0, &sim.grid_bind, &[]);
                pass.set_pipeline(&sim.scatter);
                pass.dispatch_workgroups(particles, 1, 1);
                pass.set_bind_group(0, &sim.solve_bind, &[]);
                pass.set_pipeline(&sim.density_div);
                pass.set_immediates(0, &step);
                pass.dispatch_workgroups(wide, 1, 1);
                pass.set_pipeline(&sim.div_apply);
                pass.dispatch_workgroups(wide, 1, 1);
                pass.set_pipeline(&sim.forces_eval);
                pass.dispatch_workgroups(wide, 1, 1);
                pass.set_pipeline(&sim.forces_den_apply);
                pass.dispatch_workgroups(wide, 1, 1);
                for _ in 0..refine_passes(dt_sub) {
                    pass.set_pipeline(&sim.den_kappa);
                    pass.dispatch_workgroups(wide, 1, 1);
                    pass.set_pipeline(&sim.den_apply);
                    pass.dispatch_workgroups(wide, 1, 1);
                }
                pass.set_pipeline(&sim.integrate);
                pass.dispatch_workgroups(particles, 1, 1);
            }
            pass.set_pipeline(&sim.reduce_stats);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_bind_group(0, &sim.tracer_bind, &[]);
            pass.set_pipeline(&sim.splat);
            pass.set_immediates(
                0,
                &sim::pack_step(
                    force,
                    [0.0; 3],
                    [0.0; 3],
                    dt,
                    0.0,
                    f,
                    sim::Touches::default(),
                ),
            );
            pass.dispatch_workgroups(particles, 1, 1);
            pass.set_pipeline(&sim.resolve);
            pass.dispatch_workgroups(sim.cell_groups, 1, 1);
            pass.set_pipeline(&sim.advect);
            pass.dispatch_workgroups(sim.tracer_count.div_ceil(256), 1, 1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &sim.field,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let keep = f64::from(field_keep);
            let splat = 1.0 - keep;
            pass.set_pipeline(&sim.decay);
            pass.set_blend_constant(wgpu::Color {
                r: keep,
                g: keep,
                b: keep,
                a: keep,
            });
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&sim.body);
            // The film is the glass look's oracle, so the paint is
            // solid and the field's second channel splats zero.
            pass.set_immediates(0, &pack_paint([0.0; 4], [0.0; 4], [0.0, 1.0], 0, 0.0));
            pass.set_blend_constant(wgpu::Color {
                r: splat,
                g: splat,
                b: splat,
                a: splat,
            });
            pass.set_bind_group(0, &sim.sprite_bind, &[]);
            pass.draw(0..4, 0..sim.count);
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&sim.filter);
            pass.set_bind_group(0, &sim.filter_bind, &[]);
            pass.set_immediates(
                0,
                &pack_optics(force, sim.extent, sim.field_settled, [0.0; 4], [0.0; 4]),
            );
            pass.dispatch_workgroups(sim.filter_groups[0], sim.filter_groups[1], 1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BACKDROP),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&sim.fill);
            pass.set_immediates(
                0,
                &pack_optics(force, sim.extent, sim.field_settled, [0.0; 4], [0.0; 4]),
            );
            pass.set_bind_group(0, &sim.fill_bind, &[]);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&sim.points);
            pass.set_bind_group(0, &sim.tracer_draw_bind, &[]);
            pass.draw(0..sim.tracer_count, 0..1);
        }
        encoder.copy_buffer_to_buffer(&sim.stats_src, 0, &stats, 0, STATS_BYTES);
        let sampled = f % every == 0;
        if sampled {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: WIDTH,
                    height: HEIGHT,
                    depth_or_array_layers: 1,
                },
            );
        }
        queue.submit(std::iter::once(encoder.finish()));
        stats.map_async(wgpu::MapMode::Read, .., |_| {});
        if sampled {
            readback.map_async(wgpu::MapMode::Read, .., |_| {});
        }
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        {
            let bytes = stats.get_mapped_range(..).expect("mapped");
            v_max = f32::from_le_bytes(bytes[24..28].try_into().expect("stat"));
            compr_max = compr_max.max(f32::from_le_bytes(bytes[4..8].try_into().expect("stat")));
            clamp_total = f32::from_le_bytes(bytes[36..40].try_into().expect("stat"));
        }
        stats.unmap();
        if sampled {
            let bytes = readback.get_mapped_range(..).expect("mapped");
            for (row, src) in rows
                .as_chunks_mut::<{ (WIDTH * 4) as usize }>()
                .0
                .iter_mut()
                .zip(bytes.chunks_exact(padded as usize))
            {
                row.copy_from_slice(&src[..(WIDTH * 4) as usize]);
            }
            drop(bytes);
            readback.unmap();
            sink(&rows);
        }
        if f % 240 == 0 {
            eprintln!("film: frame {f}/{frames}, {n} substeps");
        }
    }
    eprintln!(
        "compr max {compr_max:.3} % | clamps {clamp_total:.0} | v_max end {v_max:.3} | idle {idle}"
    );
    Some([WIDTH, HEIGHT])
}

fn buffer_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// The field's keep fraction at rest: a ~37 ms average, enough that
/// the rim cannot flicker frame to frame. Motion fades it to zero: a
/// 37 ms average of a surface moving at shake speed smears centimetres
/// of fog, which looks worse than the flicker it prevents.
const FIELD_KEEP: f32 = 0.8;

/// The settled interior value of the splatted field at the shipped
/// spacing, dimensionless. Measured 5.297 (this machine, 2026-08-31,
/// 600 upright frames). It is the particle layers per screen area:
/// the fluid fills the slab depth at rest density, so it scales with
/// SLAB_DEPTH over the spacing, and each Sim carries its own
/// FIELD_SETTLED * SIM_SPACING / spacing. The surface shader divides
/// by it to turn field into water thickness (M4 record, "Thickness");
/// the_settled_field_matches_the_calibration pins the constant, and
/// the_settled_field_scales_with_the_spacing pins the scaling.
const FIELD_SETTLED: f32 = 5.3;

/// Peak glint output; two percent of it — the Fresnel floor — is what
/// reaches the screen.
const SUN: f32 = 60.0;

/// Schlick reflectance at normal incidence, mirrored by F0 in
/// sim_surface.wgsl.
const GLINT_F0: f32 = 0.02;

/// The Optics immediates block in sim_surface.wgsl: two 16-byte vec3
/// slots with scalars packed into their tails, then the flat look's
/// two colours as vec4s. The lighting that is uniform across a frame —
/// world up, the glint half vector, and the folded gain — is computed
/// here once instead of per pixel. The surface reads a ramp's metric
/// from the field itself, already normalised by the splat, so no lens
/// or range reaches this block.
fn pack_optics(
    force: [f32; 3],
    extent: [f32; 2],
    field_settled: f32,
    flat: [f32; 4],
    high: [f32; 4],
) -> [u8; 80] {
    let norm = |v: [f32; 3], floor: f32| -> ([f32; 3], f32) {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(floor);
        ([v[0] / l, v[1] / l, v[2] / l], l)
    };
    // World up from the filtered force; under a metre per second
    // squared the box is near free fall and the sky holds still.
    let (mg, gl) = norm(force, 1e-6);
    let up = if gl > 1.0 {
        [-mg[0], -mg[1], -mg[2]]
    } else {
        [0.0, 0.0, 1.0]
    };
    // The light hangs over the viewer's shoulder, pinned to world up.
    let (light, _) = norm([up[0], up[1], up[2] + 0.8], 1e-6);
    // Half vector against the fixed view (0, 0, -1). Face down, light
    // meets the view head on; the floored divide keeps the glint zero
    // instead of a NaN frame.
    let (h, _) = norm([light[0], light[1], light[2] + 1.0], 1e-5);
    // An orthographic view under a directional light is degenerate on
    // flat water — every pixel hits the glint angle at once, and
    // face-up is exactly that pose. The viewer's own head shades it:
    // the sun fades out as up aligns with the view axis. All the way
    // out: any floor lets the lobe resolve the particle lattice on a
    // one-layer sheet as a honeycomb of florets.
    let s = ((up[2] - 0.85) / 0.13).clamp(0.0, 1.0);
    let fade = 1.0 - s * s * (3.0 - 2.0 * s);
    let schlick = GLINT_F0 + (1.0 - GLINT_F0) * (1.0 - h[2]).powi(5);
    let mut raw = [0u8; 80];
    for (slot, v) in [
        up[0],
        up[1],
        up[2],
        field_settled,
        extent[0],
        extent[1],
        sim::SLAB_DEPTH,
        SUN * schlick * fade,
        h[0],
        h[1],
        h[2],
    ]
    .into_iter()
    .enumerate()
    {
        raw[slot * 4..slot * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in flat.into_iter().chain(high).enumerate() {
        raw[48 + i * 4..52 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    raw
}

/// The scalar a ramp paints with. Every one is a field the solver
/// already carries, so a lens costs no pass of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lens {
    /// Speed, metres per second.
    Velocity,
    /// The whole velocity change over a substep, metres per second
    /// squared: body force, viscosity, tension and pressure together,
    /// as a running mean.
    Acceleration,
    /// The solver's pressure, pascals, as a running mean.
    Pressure,
    /// How crowded a particle's neighbourhood is — its density against
    /// rest density. Jack's word for it, 2026-09-02.
    Proximity,
    /// Which way the water goes, as a hue around the wheel, taking
    /// over from the low colour as it speeds up. Jack's pick,
    /// 2026-09-02, in place of temperature: a box that warms by
    /// millionths of a degree paints float noise, and neighbouring
    /// particles carry no shared history to correlate.
    Direction,
}

impl Lens {
    /// The order `fluid_renderer_set_look` numbers them in, and the
    /// order the `lens_at` switch in sim_sprites.wgsl reads.
    fn code(self) -> u32 {
        match self {
            Lens::Velocity => 0,
            Lens::Acceleration => 1,
            Lens::Pressure => 2,
            Lens::Proximity => 3,
            Lens::Direction => 4,
        }
    }

    /// The two ends the frame itself set, from the reduce_stats block
    /// in sim_solve.wgsl. Jack, 2026-09-02: "The gradient should go
    /// from the lowest value actually present in the sim to the
    /// highest actually present." The wheel ramps its hue in over
    /// speed, so it reads the velocity lens's pair.
    fn ends(self, stats: &[f32; STATS]) -> [f32; 2] {
        match self {
            Lens::Velocity | Lens::Direction => [stats[11], stats[6]],
            Lens::Acceleration => [stats[12], stats[13]],
            Lens::Pressure => [stats[4], stats[5]],
            Lens::Proximity => [stats[2], stats[3]],
        }
    }

    /// The narrowest span worth a ramp, in the lens's own units. Water
    /// whose spread is under it is uniform as far as the eye goes, and
    /// stretching a ramp across the solver's own noise paints confetti:
    /// a settled pool's speeds span 0.02 m/s and every particle walks
    /// the whole of that every frame (measured 2026-09-02, this
    /// machine). Three of the four are the quantity one particle
    /// spacing of water carries, so each scales with the ladder.
    fn floor(self, spacing: f32) -> f32 {
        match self {
            // The speed of a fall through one spacing. A settled pool
            // is a twentieth of it.
            Lens::Velocity | Lens::Direction => (2.0 * crate::STANDARD_GRAVITY * spacing).sqrt(),
            // The fall itself: water thrown about by less than one
            // gravity is not being thrown about.
            Lens::Acceleration => crate::STANDARD_GRAVITY,
            // The weight of one spacing of water.
            Lens::Pressure => sim::REST_DENSITY * crate::STANDARD_GRAVITY * spacing,
            // A percent of rest density; the settled body's own
            // compression spans four kilograms of it.
            Lens::Proximity => 0.01 * sim::REST_DENSITY,
        }
    }
}

/// The ends of the live lens's ramp, chased frame by frame rather than
/// taken. A frame's own extremes walk far enough to shift every colour
/// on the screen at once — a settled pool's speed ceiling moves a
/// seventh of its own span every frame (measured 2026-09-02, this
/// machine) — and the eye reads that as the water flickering.
struct Ramp {
    lens: Option<Lens>,
    ends: [f32; 2],
}

impl Ramp {
    /// Opening out is quick, so a splash has its colours within a
    /// third of a second; closing in is slow, so the palette does not
    /// breathe every time the fastest particle slows down.
    const OPEN_TAU: f32 = 0.15;
    const CLOSE_TAU: f32 = 0.6;

    fn new() -> Self {
        Self {
            lens: None,
            ends: [0.0; 2],
        }
    }

    /// None until the first stats readback lands: the block starts
    /// zeroed, and a ramp built on it would paint the whole box one
    /// end of itself. Density is the test, since no water has none.
    fn follow(
        &mut self,
        lens: Lens,
        stats: &[f32; STATS],
        spacing: f32,
        dt: f32,
    ) -> Option<[f32; 2]> {
        if stats[3] <= 0.0 {
            return None;
        }
        let floor = lens.floor(spacing);
        let mut want = lens.ends(stats);
        want[1] = want[1].max(want[0] + floor);
        if self.lens != Some(lens) {
            self.lens = Some(lens);
            self.ends = want;
            return Some(want);
        }
        for (i, (end, want)) in self.ends.iter_mut().zip(want).enumerate() {
            // The low end opens by falling and the high end by rising.
            let opening = if i == 0 { want < *end } else { want > *end };
            let tau = if opening {
                Self::OPEN_TAU
            } else {
                Self::CLOSE_TAU
            };
            *end += (want - *end) * (1.0 - (-dt / tau).exp());
        }
        // Both ends closing in at once can otherwise cross.
        self.ends[1] = self.ends[1].max(self.ends[0] + floor);
        Some(self.ends)
    }
}

/// How a look is coloured. Jack, 2026-09-02: "the user should be able
/// to choose a simple single colour or optionally two colours denoting
/// low->high values." Components are 0 to 1 as the picker shows them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    Solid([f32; 3]),
    Ramp {
        low: [f32; 3],
        high: [f32; 3],
        lens: Lens,
    },
}

/// How the water is drawn (M5 record): liquid glass over the dazzle
/// wall (the M4 look), the flat surface painted on black and nothing
/// between, or the particles alone, each a disc of the paint on black.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    Glass,
    Flat(Paint),
    Particles(Paint),
}

/// sRGB's transfer function inverted. The surface is Bgra8UnormSrgb
/// (the reference device, 2026-09-02): the shaders work in linear
/// light and the hardware encodes on write, so a picked colour goes
/// in linear and comes out as the bytes that were picked.
fn linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Slots in the reduce_stats block of sim_solve.wgsl, one f32 each.
/// The shader is the authority on what each one holds.
const STATS: usize = 14;
const STATS_BYTES: u64 = STATS as u64 * 4;

/// The smallest a disc of the particle view may draw, in pixels of
/// the drawable. Jack, 2026-09-02: "They should never be 1px".
const MIN_DISC_PX: f32 = 3.0;

/// The Paint immediates block in sim_sprites.wgsl, which the disc
/// draw and the body splat share: the two colours in linear light,
/// then the lens with the ends of its ramp, then the floor on a disc's
/// radius in metres, which the disc mixes towards as a particle's
/// neighbourhood thins. The splat ignores that last one; the disc
/// ignores nothing.
fn pack_paint(low: [f32; 4], high: [f32; 4], range: [f32; 2], lens: u32, r_min: f32) -> [u8; 48] {
    let mut raw = [0u8; 48];
    for (i, v) in low.into_iter().chain(high).enumerate() {
        raw[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    raw[32..36].copy_from_slice(&lens.to_le_bytes());
    raw[36..40].copy_from_slice(&range[0].to_le_bytes());
    raw[40..44].copy_from_slice(&range[1].to_le_bytes());
    raw[44..48].copy_from_slice(&r_min.to_le_bytes());
    raw
}

/// What the shaders need to know about a look: the low colour with a
/// one in w for either flat look, the high colour whose w says how the
/// body is painted (0 the low colour alone, 1 the ramp, 2 the
/// direction wheel), the lens code, and the ends of its ramp. The
/// glass is all zeros, which is how every shader knows it is drawing
/// glass.
#[derive(Clone, Copy)]
struct Painted {
    low: [f32; 4],
    high: [f32; 4],
    lens: u32,
    range: [f32; 2],
}

impl Painted {
    /// `ends` is the live ramp, or None where there is no ramp to
    /// draw: a solid colour, the glass, or a lens whose field has not
    /// been read back yet.
    fn new(look: Look, ends: Option<[f32; 2]>) -> Painted {
        let paint = match look {
            Look::Glass => {
                return Painted {
                    low: [0.0; 4],
                    high: [0.0; 4],
                    lens: 0,
                    range: [0.0, 1.0],
                };
            }
            Look::Flat(p) | Look::Particles(p) => p,
        };
        let colour = |c: [f32; 3], w: f32| [linear(c[0]), linear(c[1]), linear(c[2]), w];
        match (paint, ends) {
            (Paint::Ramp { low, high, lens }, Some(range)) => Painted {
                low: colour(low, 1.0),
                // The wheel paints its own colours over the whole
                // ramp, so the high colour goes unread and its slot
                // carries the two apart instead.
                high: colour(high, if lens == Lens::Direction { 2.0 } else { 1.0 }),
                lens: lens.code(),
                range,
            },
            (Paint::Solid(c), _) | (Paint::Ramp { low: c, .. }, None) => Painted {
                low: colour(c, 1.0),
                high: [0.0; 4],
                lens: 0,
                range: [0.0, 1.0],
            },
        }
    }
}

/// Adaptive low-pass on the body force. Accelerometer noise
/// (~0.15 m/s^2 RMS) pumps the pool hard enough that it never
/// settles — v_max parks at the CFL clamp and the clamp fires tens of
/// thousands of times a second at rest, which reads as jumping. The
/// blend opens with the deviation, so a real tilt or shake passes at
/// full rate while stationary noise meets an ~80 ms time constant.
struct ForceFilter {
    smooth: [f32; 3],
    started: bool,
}

impl ForceFilter {
    fn new() -> Self {
        Self {
            smooth: [0.0; 3],
            started: false,
        }
    }

    /// Returns the smoothed force and the raw-to-smooth deviation that
    /// chose the blend; the idle gate keys its wake test on the same
    /// deviation.
    fn apply(&mut self, force: [f32; 3]) -> ([f32; 3], f32) {
        if !self.started {
            self.started = true;
            self.smooth = force;
        }
        let dev = self
            .smooth
            .iter()
            .zip(&force)
            .map(|(s, f)| (f - s).powi(2))
            .sum::<f32>()
            .sqrt();
        // The floor is the still-phase cutoff (~0.4 Hz at 120 Hz), and
        // it is the rest-noise root fix (M3 record, "The noise,
        // found"): desk-still sensor noise is 0.02-0.08 m/s^2 per
        // axis, and at the old 0.1 floor (~1.9 Hz) enough reached the
        // fluid's 3.3-3.7 Hz slosh band to pump the ungripped lattice
        // past the 0.05 m/s dot cutoff. Real motion spikes dev and
        // lifts alpha within a frame, so only true stillness is
        // filtered this hard.
        let alpha = (dev / 2.0).clamp(0.02, 1.0);
        for (s, f) in self.smooth.iter_mut().zip(force) {
            *s += (f - *s) * alpha;
        }
        (self.smooth, dev)
    }
}

/// Smooths the gyroscope and differentiates it for the Euler term. The
/// gyro is far cleaner than the accelerometer, so the smoothing is
/// light; the derivative is clamped against flick spikes and zeroed
/// across pauses, where the frame gap is not a rotation interval.
/// The fingers on the glass, turned into the drag the solver reads.
/// The shell reports where each presses in normalised drawable
/// coordinates and which slot it holds — the facts only the shell has,
/// since only it can tell one finger from another. Every metre and
/// metre per second below is computed here (D6).
struct Fingers {
    at: [Option<[f32; 2]>; sim::MAX_TOUCHES],
    world: [[f32; 2]; sim::MAX_TOUCHES],
    velocity: [[f32; 2]; sim::MAX_TOUCHES],
    tracked: [bool; sim::MAX_TOUCHES],
}

impl Fingers {
    /// How far into the water a finger drags, world metres. The world
    /// is WORLD_SCALE times the device, so this is 25 mm of glass: a
    /// fingertip and the wider net Jack asked for around it
    /// (2026-09-02).
    const RADIUS: f32 = crate::WORLD_SCALE * 0.025;

    /// Touch samples and frames run at the same rate but out of phase,
    /// so the raw per-frame difference alternates between a doubled
    /// step and none. A 20 ms average evens that out well inside the
    /// drag's own 25 ms response.
    const SMOOTH_TAU: f32 = 0.02;

    fn new() -> Self {
        Self {
            at: [None; sim::MAX_TOUCHES],
            world: [[0.0; 2]; sim::MAX_TOUCHES],
            velocity: [[0.0; 2]; sim::MAX_TOUCHES],
            tracked: [false; sim::MAX_TOUCHES],
        }
    }

    fn any_down(&self) -> bool {
        self.at.iter().any(Option::is_some)
    }

    /// This frame's drag. Each finger's velocity comes from the
    /// frame's own step, so one held still brakes the water it sits in
    /// and one lifted stops dragging entirely. The solver reads the
    /// live fingers packed from the front, so a lifted finger costs
    /// its loop nothing.
    fn step(&mut self, extent: [f32; 2], dt: f32) -> sim::Touches {
        let mut touches = sim::Touches {
            radius: Self::RADIUS,
            ..Default::default()
        };
        for slot in 0..sim::MAX_TOUCHES {
            let Some([x, y]) = self.at[slot] else {
                self.tracked[slot] = false;
                continue;
            };
            // Normalised drawable to the box plane: x runs right in
            // both, y runs down the drawable and up the box.
            let world = [(2.0 * x - 1.0) * extent[0], (1.0 - 2.0 * y) * extent[1]];
            if self.tracked[slot] && dt > 0.0 {
                let alpha = 1.0 - (-dt / Self::SMOOTH_TAU).exp();
                let was = self.world[slot];
                for ((smooth, now), then) in self.velocity[slot].iter_mut().zip(world).zip(was) {
                    *smooth += ((now - then) / dt - *smooth) * alpha;
                }
            } else {
                self.velocity[slot] = [0.0; 2];
            }
            self.world[slot] = world;
            self.tracked[slot] = true;
            touches.each[touches.count as usize] = sim::Touch {
                at: world,
                velocity: self.velocity[slot],
            };
            touches.count += 1;
        }
        touches
    }
}

struct RotationTracker {
    smooth: [f32; 3],
    primed: bool,
}

impl RotationTracker {
    fn new() -> Self {
        Self {
            smooth: [0.0; 3],
            primed: false,
        }
    }

    /// Returns the smoothed angular velocity and its time derivative.
    fn apply(&mut self, omega: [f32; 3], dt: f32) -> ([f32; 3], [f32; 3]) {
        if !self.primed || dt <= 0.0 || dt > 0.1 {
            self.primed = true;
            self.smooth = omega;
            return (omega, [0.0; 3]);
        }
        let alpha = 1.0 - (-dt / 0.025).exp();
        let prev = self.smooth;
        for (s, o) in self.smooth.iter_mut().zip(omega) {
            *s += (o - *s) * alpha;
        }
        let domega =
            std::array::from_fn(|i| ((self.smooth[i] - prev[i]) / dt).clamp(-200.0, 200.0));
        (self.smooth, domega)
    }
}

/// Sleeps the simulation once the pool has settled and nothing moves the
/// phone: a still phone encodes no GPU work at all ("idle costs nothing").
/// The gate runs every tick, asleep or not — the filter it reads must stay
/// live, or the wake tests would compare frozen numbers. Deviation catches
/// a shake in one tick; the angle and magnitude tests against the force
/// snapshot taken at sleep catch a slow tilt the deviation never sees.
struct IdleGate {
    still: u32,
    sleep_force: Option<[f32; 3]>,
}

impl IdleGate {
    /// Device rest v_max: 0.02 settled at the shipped 4x scale, 0.03..0.12
    /// wandering at 1x under sensor noise (both 2026-08-31); the threshold
    /// sits between the 4x rest and real motion.
    // Just under the draw's 0.05 m/s dot-blanking cutoff: the gate may
    // only freeze a picture that already shows nothing moving. At the
    // old 0.12 the flat-pose bead — which translates under tension —
    // froze mid-wander (Jack, 2026-08-31).
    const V_SLEEP: f32 = 0.04;
    const DEV_SLEEP: f32 = 0.5;
    const STILL_FRAMES: u32 = 180;
    const DEV_WAKE: f32 = 1.2;
    /// cos 1.5 degrees: sensor noise wanders the smoothed force ~0.2
    /// degrees, a real tilt crosses this within two idle ticks.
    const COS_WAKE: f32 = 0.999_657;
    const MAG_WAKE: f32 = 0.3;
    /// Desk-still gyro noise is under 0.01 rad/s; a deliberate turn
    /// crosses this in a tick. Rotation moves water that the force
    /// tests cannot see - a flat phone spun about its normal holds
    /// gravity fixed in the box frame.
    const OMEGA_WAKE: f32 = 0.05;

    fn new() -> Self {
        Self {
            still: 0,
            sleep_force: None,
        }
    }

    fn sleeping(&self) -> bool {
        self.sleep_force.is_some()
    }

    /// True while the frame may be skipped.
    fn asleep(
        &mut self,
        touched: bool,
        smooth: [f32; 3],
        dev: f32,
        omega: [f32; 3],
        v_max: f32,
    ) -> bool {
        fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
            a.iter().zip(&b).map(|(x, y)| x * y).sum()
        }
        if touched {
            self.sleep_force = None;
            self.still = 0;
            return false;
        }
        let spinning = dot(omega, omega).sqrt() > Self::OMEGA_WAKE;
        if let Some(rest) = self.sleep_force {
            let cos = dot(smooth, rest) / (dot(smooth, smooth) * dot(rest, rest)).sqrt().max(1e-6);
            let mag_shift = (dot(smooth, smooth).sqrt() - dot(rest, rest).sqrt()).abs();
            if dev > Self::DEV_WAKE
                || cos < Self::COS_WAKE
                || mag_shift > Self::MAG_WAKE
                || spinning
            {
                self.sleep_force = None;
                self.still = 0;
                return false;
            }
            return true;
        }
        if v_max < Self::V_SLEEP && dev < Self::DEV_SLEEP && !spinning {
            self.still += 1;
            if self.still >= Self::STILL_FRAMES {
                self.sleep_force = Some(smooth);
                return true;
            }
        } else {
            self.still = 0;
        }
        false
    }
}

/// Rest keep down to zero across the hand-tremor..shake band of v_max.
fn field_keep_target(v_max: f32) -> f32 {
    let t = ((v_max - 0.05) / 0.20).clamp(0.0, 1.0);
    FIELD_KEEP * (1.0 - t * t * (3.0 - 2.0 * t))
}

const DECAY: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::Constant,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::Constant,
        operation: wgpu::BlendOperation::Add,
    },
};

const OVER: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// The body splat, scaled by the blend constant: the pass sets it to
/// 1 - keep, the complement of the decay draw, so decay and splat
/// always sum to an exact average.
const SPLAT: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Constant,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Constant,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

struct Particles {
    params: wgpu::Buffer,
    integrate: wgpu::ComputePipeline,
    sprites: wgpu::RenderPipeline,
    integrate_bind: wgpu::BindGroup,
    sprite_bind: wgpu::BindGroup,
    count: u32,
    radius: f32,
    extent: [f32; 2],
}

impl Particles {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        count: u32,
        radius: f32,
        extent: [f32; 2],
    ) -> Particles {
        // A radius near the half-extent turns the wall clamp inside out; no
        // sane sprite approaches a fifth of the box.
        let radius = radius.min(0.2 * extent[0].min(extent[1]));
        // The buffer handle is dropped here; the bind groups keep the
        // resource alive.
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: u64::from(count) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        let mut seeded = Vec::with_capacity(count as usize * 16);
        for v in particles::seed(count, extent) {
            seeded.extend_from_slice(&v.to_le_bytes());
        }
        buffer
            .get_mapped_range_mut(..)
            .expect("mapped at creation")
            .copy_from_slice(&seeded);
        buffer.unmap();

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let integrate_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("integrate"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ),
                buffer_entry(
                    1,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Uniform,
                ),
            ],
        });
        let sprite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprites"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer_entry(
                    1,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BufferBindingType::Uniform,
                ),
            ],
        });
        let bind = |layout: &wgpu::BindGroupLayout| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: params.as_entire_binding(),
                    },
                ],
            })
        };
        let pipeline_layout = |layout: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            })
        };

        let integrate_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("integrate"),
            source: wgpu::ShaderSource::Wgsl(include_str!("integrate.wgsl").into()),
        });
        let sprites_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprites"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprites.wgsl").into()),
        });

        let integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("integrate"),
            layout: Some(&pipeline_layout(&integrate_layout)),
            module: &integrate_module,
            entry_point: None,
            compilation_options: Default::default(),
            cache: None,
        });
        let sprites = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprites"),
            layout: Some(&pipeline_layout(&sprite_layout)),
            vertex: wgpu::VertexState {
                module: &sprites_module,
                entry_point: None,
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sprites_module,
                entry_point: None,
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(ADDITIVE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let integrate_bind = bind(&integrate_layout);
        let sprite_bind = bind(&sprite_layout);
        Particles {
            params,
            integrate,
            sprites,
            integrate_bind,
            sprite_bind,
            count,
            radius,
            extent,
        }
    }
}

/// Knobs the shell passes through `fluid_renderer_create`. A nonzero
/// `bench_sweeps` replaces the demo with the M3 stage-0 microbench.
pub struct RenderOptions {
    pub particle_count: u32,
    pub sprite_radius: f32,
    pub bench_sweeps: u32,
    pub bench_spacing: f32,
    pub sim_substeps: u32,
    pub tracers: u32,
    /// The fluid's particle count as a multiple of the shipped lattice;
    /// `spacing_for` turns it into a spacing unless `bench_spacing`
    /// pins one.
    pub particle_scale: f32,
}

enum Mode {
    Demo(Box<Particles>),
    Bench(Box<Bench>),
    Sim(Box<Sim>),
}

/// The M3 record's 2.5 mm spacing times the world scale — the same
/// on-screen resolution at every scale. The 1x of the particle ladder.
const SIM_SPACING: f32 = crate::WORLD_SCALE * 0.0025;

/// The spacing that seeds nearest `scale` times the 1x count. The
/// lattice quantises in whole rows and layers, so the cube root misses
/// badly at the ends (0.25x would seed 288 particles, 16x 32,340); the
/// search walks the FLUID_SPACING clamp range in steps of half a
/// percent of SIM_SPACING. One count spans a run of spacings, and the
/// run's spacing nearest the cube root wins, so 1x is SIM_SPACING
/// itself.
fn spacing_for(scale: f32, extent: [f32; 2]) -> f32 {
    let count = |spacing: f32| {
        sim::lattice_dims(spacing, extent, 0.5)
            .iter()
            .product::<u32>()
    };
    let target = scale * count(SIM_SPACING) as f32;
    let ideal = SIM_SPACING / scale.cbrt();
    let mut best = (f32::INFINITY, f32::INFINITY, SIM_SPACING);
    for permille in (400..=4000).step_by(5) {
        let spacing = SIM_SPACING * permille as f32 / 1000.0;
        let miss = (count(spacing) as f32 / target).ln().abs();
        let off = (spacing - ideal).abs();
        if (miss, off) < (best.0, best.1) {
            best = (miss, off, spacing);
        }
    }
    best.2
}

struct Sim {
    count_cells: wgpu::ComputePipeline,
    scan_single: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    density_div: wgpu::ComputePipeline,
    forces_eval: wgpu::ComputePipeline,
    forces_den_apply: wgpu::ComputePipeline,
    div_apply: wgpu::ComputePipeline,
    den_kappa: wgpu::ComputePipeline,
    den_apply: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    reduce_stats: wgpu::ComputePipeline,
    sprites: wgpu::RenderPipeline,
    splat: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
    advect: wgpu::ComputePipeline,
    points: wgpu::RenderPipeline,
    discs: wgpu::RenderPipeline,
    body: wgpu::RenderPipeline,
    body_flow: wgpu::RenderPipeline,
    decay: wgpu::RenderPipeline,
    fill: wgpu::RenderPipeline,
    fill_layout: wgpu::BindGroupLayout,
    filter_layout: wgpu::BindGroupLayout,
    field_sampler: wgpu::Sampler,
    field: wgpu::TextureView,
    filter: wgpu::ComputePipeline,
    flow: wgpu::TextureView,
    filtered: wgpu::TextureView,
    filter_bind: wgpu::BindGroup,
    filter_groups: [u32; 2],
    fill_bind: wgpu::BindGroup,
    tracer_bind: wgpu::BindGroup,
    tracer_draw_bind: wgpu::BindGroup,
    grid_bind: wgpu::BindGroup,
    scan_bind: wgpu::BindGroup,
    solve_bind: wgpu::BindGroup,
    sprite_bind: wgpu::BindGroup,
    stats_src: wgpu::Buffer,
    stats_staging: [StagingSlot; 3],
    stats: [f32; STATS],
    spacing: f32,
    field_settled: f32,
    extent: [f32; 2],
    tracer_count: u32,
    count: u32,
    cell_groups: u32,
    max_substeps: u32,
    substeps_used: u32,
    field_keep: f32,
    // Both fields accumulate across frames, so a frame that skipped
    // one, and a texture just built, leave it nothing to keep.
    field_live: bool,
    flow_live: bool,
    frame_seed: u32,
    #[cfg(test)]
    tracers: wgpu::Buffer,
    #[cfg(test)]
    density: wgpu::Buffer,
    #[cfg(test)]
    positions: wgpu::Buffer,
    #[cfg(test)]
    velocities: wgpu::Buffer,
    #[cfg(test)]
    prev_vel: wgpu::Buffer,
}

impl Sim {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        extent: [f32; 2],
        substeps: u32,
        spacing: f32,
        tracer_count: u32,
        surface: [u32; 2],
    ) -> Sim {
        let h = 1.2 * spacing;
        let grid = sim::Grid::new(extent, 2.0 * h);
        let cells = grid.cell_count();
        let seeded = sim::seed_slab(spacing, extent, 0.5);
        let count = (seeded.len() / 4) as u32;
        eprintln!("sim: {count} particles, {cells} cells, cap {substeps}, spacing {spacing} m");

        let storage = |label: &str, size: u64, extra: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | extra,
                mapped_at_creation: false,
            })
        };
        let none = wgpu::BufferUsages::empty();
        // The tests read the particles straight back; a shipped frame
        // never copies them off the GPU.
        let readback = if cfg!(test) {
            wgpu::BufferUsages::COPY_SRC
        } else {
            none
        };
        let positions = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim positions"),
            size: u64::from(count) * 16,
            usage: wgpu::BufferUsages::STORAGE | readback,
            mapped_at_creation: true,
        });
        let mut bytes = Vec::with_capacity(seeded.len() * 4);
        for v in &seeded {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        positions
            .get_mapped_range_mut(..)
            .expect("mapped at creation")
            .copy_from_slice(&bytes);
        positions.unmap();
        // wgpu zero-initialises buffers: the fluid starts at rest.
        let velocities = storage("sim velocities", u64::from(count) * 16, readback);
        let counts = storage("sim counts", u64::from(cells) * 4, none);
        // One slot past the last cell holds the total: a sweep reads a
        // cell's end as the next cell's start.
        let starts = storage("sim starts", u64::from(cells + 1) * 4, none);
        let cursors = storage("sim cursors", u64::from(cells) * 4, none);
        let sorted = storage("sim sorted", u64::from(count) * 4, none);
        let density = storage(
            "sim density",
            u64::from(count) * 4,
            wgpu::BufferUsages::COPY_SRC,
        );
        let alpha = storage("sim alpha", u64::from(count) * 4, none);
        let kd = storage("sim kappa over density", u64::from(count) * 4, none);
        let pressure = storage("sim pressure", u64::from(count) * 4, none);
        let prev_pressure = storage("sim prev pressure", u64::from(count) * 4, none);
        let clamps = storage("sim clamps", 4, none);
        let accel = storage("sim accel", u64::from(count) * 16, none);
        let xsph = storage("sim xsph", u64::from(count) * 16, none);
        let nbr = storage("sim neighbours", u64::from(count * NBR_CAP) * 16, none);
        let nbr_n = storage("sim neighbour counts", u64::from(count) * 4, none);
        let nbr_over = storage("sim neighbour overflow", 4, none);
        let wall_grad = storage("sim wall gradients", u64::from(count) * 16, none);
        let prev_vel = storage("sim previous velocities", u64::from(count) * 16, readback);
        let stats_src = storage("sim stats", STATS_BYTES, wgpu::BufferUsages::COPY_SRC);
        // The box starts at the lab constants' temperature, 20 C.
        let temperature = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim temperature"),
            size: u64::from(count) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        let mut warm = Vec::with_capacity(count as usize * 4);
        for _ in 0..count {
            warm.extend_from_slice(&293.15f32.to_le_bytes());
        }
        temperature
            .get_mapped_range_mut(..)
            .expect("mapped at creation")
            .copy_from_slice(&warm);
        temperature.unmap();
        let vel_grid = storage("sim vel grid", u64::from(cells) * 16, none);
        let vel_flat = storage("sim vel flat", u64::from(cells) * 16, none);
        let tracers = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim tracers"),
            size: u64::from(tracer_count.max(1)) * 8,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        if tracer_count > 0 {
            let seeded_tracers = sim::seed_tracers(tracer_count, extent, 0.5);
            let mut tracer_bytes = Vec::with_capacity(seeded_tracers.len() * 4);
            for v in &seeded_tracers {
                tracer_bytes.extend_from_slice(&v.to_le_bytes());
            }
            tracers
                .get_mapped_range_mut(..)
                .expect("mapped at creation")
                .copy_from_slice(&tracer_bytes);
        }
        tracers.unmap();
        let stats_staging = std::array::from_fn(|_| StagingSlot {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sim stats staging"),
                size: STATS_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            state: Arc::new(AtomicU8::new(SLOT_FREE)),
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim params"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        params
            .get_mapped_range_mut(..)
            .expect("mapped at creation")
            .copy_from_slice(&sim::pack_sim_params(
                &grid,
                count,
                h,
                sim::REST_DENSITY * spacing * spacing * spacing,
            ));
        params.unmap();

        let compute = wgpu::ShaderStages::COMPUTE;
        let vertex = wgpu::ShaderStages::VERTEX;
        let uniform = |b| buffer_entry(b, compute, wgpu::BufferBindingType::Uniform);
        let ro = |b| {
            buffer_entry(
                b,
                compute,
                wgpu::BufferBindingType::Storage { read_only: true },
            )
        };
        let rw = |b| {
            buffer_entry(
                b,
                compute,
                wgpu::BufferBindingType::Storage { read_only: false },
            )
        };
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let grid_layout = layout("sim grid", &[uniform(0), ro(1), rw(2), ro(3), rw(4), rw(5)]);
        // scan_single never touches block_sums; the gap at 3 stays open.
        let scan_layout = layout("sim scan", &[uniform(0), ro(1), rw(2), rw(4)]);
        let solve_layout = layout(
            "sim solve",
            &[
                uniform(0),
                rw(1),
                rw(2),
                rw(3),
                ro(4),
                ro(5),
                rw(6),
                rw(7),
                rw(8),
                rw(9),
                rw(10),
                rw(11),
                rw(12),
                rw(13),
                rw(14),
                rw(15),
                rw(16),
                rw(17),
                rw(18),
                rw(19),
            ],
        );
        let tracer_layout = layout(
            "sim tracers",
            &[uniform(0), ro(1), ro(2), rw(3), rw(4), rw(5)],
        );
        let tracer_draw_layout = layout(
            "sim tracer draw",
            &[
                buffer_entry(0, vertex, wgpu::BufferBindingType::Uniform),
                buffer_entry(
                    3,
                    vertex,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
            ],
        );
        let sprite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sim sprites"),
            entries: &[
                buffer_entry(0, vertex, wgpu::BufferBindingType::Uniform),
                buffer_entry(
                    1,
                    vertex,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer_entry(
                    2,
                    vertex,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer_entry(
                    4,
                    vertex,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer_entry(
                    5,
                    vertex,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
            ],
        });
        fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        let bind = |label, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries,
            })
        };
        let grid_bind = bind(
            "sim grid",
            &grid_layout,
            &[
                entry(0, &params),
                entry(1, &positions),
                entry(2, &counts),
                entry(3, &starts),
                entry(4, &cursors),
                entry(5, &sorted),
            ],
        );
        let scan_bind = bind(
            "sim scan",
            &scan_layout,
            &[
                entry(0, &params),
                entry(1, &counts),
                entry(2, &starts),
                entry(4, &cursors),
            ],
        );
        let solve_bind = bind(
            "sim solve",
            &solve_layout,
            &[
                entry(0, &params),
                entry(1, &positions),
                entry(2, &velocities),
                entry(3, &nbr_over),
                entry(4, &starts),
                entry(5, &sorted),
                entry(6, &density),
                entry(7, &alpha),
                entry(8, &kd),
                entry(9, &pressure),
                entry(10, &prev_pressure),
                entry(11, &temperature),
                entry(12, &stats_src),
                entry(13, &clamps),
                entry(14, &accel),
                entry(15, &xsph),
                entry(16, &nbr),
                entry(17, &nbr_n),
                entry(18, &wall_grad),
                entry(19, &prev_vel),
            ],
        );
        let tracer_bind = bind(
            "sim tracers",
            &tracer_layout,
            &[
                entry(0, &params),
                entry(1, &positions),
                entry(2, &velocities),
                entry(3, &vel_grid),
                entry(4, &tracers),
                entry(5, &vel_flat),
            ],
        );
        let tracer_draw_bind = bind(
            "sim tracer draw",
            &tracer_draw_layout,
            &[entry(0, &params), entry(3, &tracers)],
        );
        let sprite_bind = bind(
            "sim sprites",
            &sprite_layout,
            &[
                entry(0, &params),
                entry(1, &positions),
                entry(2, &velocities),
                entry(4, &density),
                entry(5, &prev_vel),
            ],
        );

        let module = |label, source: &'static str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let grid_module = module("sim_grid", include_str!("sim_grid.wgsl"));
        let scan_module = module("sim_scan", include_str!("sim_scan.wgsl"));
        let solve_module = module("sim_solve", include_str!("sim_solve.wgsl"));
        let sprite_module = module("sim_sprites", include_str!("sim_sprites.wgsl"));
        let tracer_module = module("sim_tracers", include_str!("sim_tracers.wgsl"));
        let pipe_layout = |label, layout: &wgpu::BindGroupLayout, immediate_size: u32| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size,
            })
        };
        let fill_layout = layout_frag(device, "sim surface");
        let field_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sim surface"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let field = field_view(device, "sim field", surface);
        let flow = field_view(device, "sim flow", surface);
        // The optics read the field through field_filter: the raw splat
        // in, the blurred thickness with its differences out, and the
        // fill pass samples that.
        let filter_layout = layout_filter(device);
        let filtered = filtered_view(device, surface);
        let fill_bind = field_bind(
            device,
            &fill_layout,
            &filtered,
            &field_sampler,
            &field,
            &flow,
        );
        let filter_bind = filter_bind(device, &filter_layout, &field, &filtered);
        let surface_module = module("sim_surface", include_str!("sim_surface.wgsl"));
        let fill_pl = pipe_layout("sim surface", &fill_layout, 80);
        let filter_pl = pipe_layout("sim field filter", &filter_layout, 80);
        let grid_pl = pipe_layout("sim grid", &grid_layout, 0);
        let scan_pl = pipe_layout("sim scan", &scan_layout, 0);
        let solve_pl = pipe_layout("sim solve", &solve_layout, sim::STEP_BYTES as u32);
        let sprite_pl = pipe_layout("sim sprites", &sprite_layout, 0);
        let tracer_pl = pipe_layout("sim tracers", &tracer_layout, sim::STEP_BYTES as u32);
        let tracer_draw_pl = pipe_layout("sim tracer draw", &tracer_draw_layout, 0);
        let paint_pl = pipe_layout("sim paint", &sprite_layout, 48);
        let pipeline =
            |layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, entry_point: &str| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry_point),
                    layout: Some(layout),
                    module,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };

        Sim {
            count_cells: pipeline(&grid_pl, &grid_module, "count"),
            scan_single: pipeline(&scan_pl, &scan_module, "scan_single"),
            scatter: pipeline(&grid_pl, &grid_module, "scatter"),
            density_div: pipeline(&solve_pl, &solve_module, "density_div"),
            forces_eval: pipeline(&solve_pl, &solve_module, "forces_eval"),
            forces_den_apply: pipeline(&solve_pl, &solve_module, "forces_den_apply"),
            div_apply: pipeline(&solve_pl, &solve_module, "div_apply"),
            den_kappa: pipeline(&solve_pl, &solve_module, "den_kappa"),
            den_apply: pipeline(&solve_pl, &solve_module, "den_apply"),
            integrate: pipeline(&solve_pl, &solve_module, "integrate"),
            reduce_stats: pipeline(&solve_pl, &solve_module, "reduce_stats"),
            sprites: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sim sprites"),
                layout: Some(&sprite_pl),
                vertex: wgpu::VertexState {
                    module: &sprite_module,
                    entry_point: Some("sprite"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_module,
                    entry_point: Some("glow"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(ADDITIVE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }),
            splat: pipeline(&tracer_pl, &tracer_module, "splat"),
            resolve: pipeline(&tracer_pl, &tracer_module, "resolve"),
            advect: pipeline(&tracer_pl, &tracer_module, "advect"),
            points: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sim points"),
                layout: Some(&tracer_draw_pl),
                vertex: wgpu::VertexState {
                    module: &sprite_module,
                    entry_point: Some("point"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::PointList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_module,
                    entry_point: Some("dot_frag"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(ADDITIVE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }),
            // Written opaque, colour only: a disc over the body is the
            // body's colour and a disc in the air is a droplet, so the
            // screen stays two colours.
            discs: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sim discs"),
                layout: Some(&paint_pl),
                vertex: wgpu::VertexState {
                    module: &sprite_module,
                    entry_point: Some("disc"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_module,
                    entry_point: Some("disc_frag"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }),
            body: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sim body"),
                layout: Some(&paint_pl),
                vertex: wgpu::VertexState {
                    module: &sprite_module,
                    entry_point: Some("body"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_module,
                    entry_point: Some("weight"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rg16Float,
                        blend: Some(SPLAT),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }),
            body_flow: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sim body flow"),
                layout: Some(&paint_pl),
                vertex: wgpu::VertexState {
                    module: &sprite_module,
                    entry_point: Some("body_flow"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_module,
                    entry_point: Some("flow"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rg16Float,
                        blend: Some(SPLAT),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }),
            decay: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sim decay"),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("sim decay"),
                        bind_group_layouts: &[],
                        immediate_size: 0,
                    }),
                ),
                vertex: wgpu::VertexState {
                    module: &surface_module,
                    entry_point: Some("fill"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &surface_module,
                    entry_point: Some("decay_frag"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rg16Float,
                        blend: Some(DECAY),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }),
            fill: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sim fill"),
                layout: Some(&fill_pl),
                vertex: wgpu::VertexState {
                    module: &surface_module,
                    entry_point: Some("fill"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &surface_module,
                    entry_point: Some("surface_frag"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(OVER),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }),
            fill_layout,
            filter_layout,
            field_sampler,
            field,
            filter: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("sim field filter"),
                layout: Some(&filter_pl),
                module: &surface_module,
                entry_point: Some("field_filter"),
                compilation_options: Default::default(),
                cache: None,
            }),
            flow,
            filtered,
            filter_bind,
            filter_groups: filter_groups(surface),
            fill_bind,
            tracer_bind,
            tracer_draw_bind,
            grid_bind,
            scan_bind,
            solve_bind,
            sprite_bind,
            stats_src,
            stats_staging,
            stats: [0.0; STATS],
            spacing,
            field_settled: FIELD_SETTLED * SIM_SPACING / spacing,
            extent,
            tracer_count,
            count,
            cell_groups: cells.div_ceil(256),
            max_substeps: substeps,
            substeps_used: 0,
            field_keep: FIELD_KEEP,
            field_live: false,
            flow_live: false,
            frame_seed: 0,
            #[cfg(test)]
            tracers,
            #[cfg(test)]
            density,
            #[cfg(test)]
            positions,
            #[cfg(test)]
            velocities,
            #[cfg(test)]
            prev_vel,
        }
    }
}

impl Sim {
    fn resize(&mut self, device: &wgpu::Device, surface: [u32; 2]) {
        self.field = field_view(device, "sim field", surface);
        self.flow = field_view(device, "sim flow", surface);
        self.filtered = filtered_view(device, surface);
        self.fill_bind = field_bind(
            device,
            &self.fill_layout,
            &self.filtered,
            &self.field_sampler,
            &self.field,
            &self.flow,
        );
        self.filter_bind = filter_bind(device, &self.filter_layout, &self.field, &self.filtered);
        self.filter_groups = filter_groups(surface);
        self.field_live = false;
        self.flow_live = false;
    }
}

/// The filtered field: blurred thickness, its two texel differences,
/// and its Laplacian, at the field's quarter resolution.
fn filtered_view(device: &wgpu::Device, surface: [u32; 2]) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("sim field filtered"),
            size: wgpu::Extent3d {
                width: (surface[0] / 4).max(1),
                height: (surface[1] / 4).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// field_filter's 16x16 tiles over the quarter-resolution field.
fn filter_groups(surface: [u32; 2]) -> [u32; 2] {
    [
        (surface[0] / 4).max(1).div_ceil(16),
        (surface[1] / 4).max(1).div_ceil(16),
    ]
}

fn filter_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    field: &wgpu::TextureView,
    filtered: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sim field filter"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(field),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(filtered),
            },
        ],
    })
}

fn layout_filter(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sim field filter"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
        ],
    })
}

fn field_view(device: &wgpu::Device, label: &str, surface: [u32; 2]) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: (surface[0] / 4).max(1),
                height: (surface[1] / 4).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // r the splatted thickness, g that thickness weighted by
            // the lens. The ratio is the flat look's ramp; the glass
            // splats a zero there and never reads it.
            format: wgpu::TextureFormat::Rg16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn field_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    field: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    splat: &wgpu::TextureView,
    flow: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sim surface"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(field),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(splat),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(flow),
            },
        ],
    })
}

fn layout_frag(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // The raw splat, for the flat look's ramp. Binding 2 is
            // the filter pass's storage view of the same texture and
            // has no place in a fragment layout.
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // The direction lens's own field.
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

struct Validation {
    seeded: Vec<f32>,
    grid: sim::Grid,
    h: f32,
    mass: f32,
    starts: wgpu::Buffer,
    density: wgpu::Buffer,
    staging: wgpu::Buffer,
}

struct Bench {
    count: wgpu::ComputePipeline,
    scan_blocks: wgpu::ComputePipeline,
    scan_sums: wgpu::ComputePipeline,
    add_back: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    density_sweep: wgpu::ComputePipeline,
    grid_bind: wgpu::BindGroup,
    scan_bind: wgpu::BindGroup,
    density_bind: wgpu::BindGroup,
    sweeps: u32,
    particle_groups: u32,
    cell_groups: u32,
    validation: Option<Validation>,
}

impl Bench {
    fn new(device: &wgpu::Device, extent: [f32; 2], options: &RenderOptions) -> Bench {
        let spacing = if options.bench_spacing > 0.0 {
            options.bench_spacing
        } else {
            0.8 * SIM_SPACING
        }
        .clamp(0.4 * SIM_SPACING, 4.0 * SIM_SPACING);
        let h = 1.2 * spacing;
        let grid = sim::Grid::new(extent, 2.0 * h);
        let cells = grid.cell_count();
        // The scan's second pass is one workgroup over the block sums.
        assert!(cells <= 65_536, "the scan handles 256 blocks of 256");
        let seeded = sim::seed_slab(spacing, extent, 0.5);
        let count = (seeded.len() / 4) as u32;
        let mass = sim::REST_DENSITY * spacing * spacing * spacing;
        eprintln!(
            "bench: {count} particles, {cells} cells, {} sweeps, spacing {spacing} m",
            options.bench_sweeps
        );

        let storage = |label: &str, size: u64, extra: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | extra,
                mapped_at_creation: false,
            })
        };
        let none = wgpu::BufferUsages::empty();
        let positions = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("positions"),
            size: u64::from(count) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        let mut bytes = Vec::with_capacity(seeded.len() * 4);
        for v in &seeded {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        positions
            .get_mapped_range_mut(..)
            .expect("mapped at creation")
            .copy_from_slice(&bytes);
        positions.unmap();
        let density = storage(
            "density",
            u64::from(count) * 4,
            wgpu::BufferUsages::COPY_SRC,
        );
        let counts = storage("counts", u64::from(cells) * 4, none);
        let starts = storage("starts", u64::from(cells) * 4, wgpu::BufferUsages::COPY_SRC);
        let cursors = storage("cursors", u64::from(cells) * 4, none);
        let block_sums = storage("block_sums", u64::from(cells.div_ceil(256)) * 4, none);
        let sorted = storage("sorted", u64::from(count) * 4, none);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim params"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        params
            .get_mapped_range_mut(..)
            .expect("mapped at creation")
            .copy_from_slice(&sim::pack_sim_params(&grid, count, h, mass));
        params.unmap();
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bench staging"),
            size: u64::from(cells.max(count)) * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let compute = wgpu::ShaderStages::COMPUTE;
        let uniform = |b| buffer_entry(b, compute, wgpu::BufferBindingType::Uniform);
        let ro = |b| {
            buffer_entry(
                b,
                compute,
                wgpu::BufferBindingType::Storage { read_only: true },
            )
        };
        let rw = |b| {
            buffer_entry(
                b,
                compute,
                wgpu::BufferBindingType::Storage { read_only: false },
            )
        };
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let grid_layout = layout("grid", &[uniform(0), ro(1), rw(2), ro(3), rw(4), rw(5)]);
        let scan_layout = layout("scan", &[uniform(0), ro(1), rw(2), rw(3), rw(4)]);
        let density_layout = layout("density", &[uniform(0), ro(1), ro(2), ro(3), ro(4), rw(5)]);
        fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        let bind = |label, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries,
            })
        };
        let grid_bind = bind(
            "grid",
            &grid_layout,
            &[
                entry(0, &params),
                entry(1, &positions),
                entry(2, &counts),
                entry(3, &starts),
                entry(4, &cursors),
                entry(5, &sorted),
            ],
        );
        let scan_bind = bind(
            "scan",
            &scan_layout,
            &[
                entry(0, &params),
                entry(1, &counts),
                entry(2, &starts),
                entry(3, &block_sums),
                entry(4, &cursors),
            ],
        );
        let density_bind = bind(
            "density",
            &density_layout,
            &[
                entry(0, &params),
                entry(1, &positions),
                entry(2, &cursors),
                entry(3, &starts),
                entry(4, &sorted),
                entry(5, &density),
            ],
        );

        let module = |label, source: &'static str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let grid_module = module("sim_grid", include_str!("sim_grid.wgsl"));
        let scan_module = module("sim_scan", include_str!("sim_scan.wgsl"));
        let density_module = module("sim_density", include_str!("sim_density.wgsl"));
        let pipe_layout = |layout: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            })
        };
        let grid_pl = pipe_layout(&grid_layout);
        let scan_pl = pipe_layout(&scan_layout);
        let density_pl = pipe_layout(&density_layout);
        let pipeline =
            |layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, entry_point: &str| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry_point),
                    layout: Some(layout),
                    module,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };

        Bench {
            count: pipeline(&grid_pl, &grid_module, "count"),
            scan_blocks: pipeline(&scan_pl, &scan_module, "scan_blocks"),
            scan_sums: pipeline(&scan_pl, &scan_module, "scan_sums"),
            add_back: pipeline(&scan_pl, &scan_module, "add_back"),
            scatter: pipeline(&grid_pl, &grid_module, "scatter"),
            density_sweep: pipeline(&density_pl, &density_module, "density_sweep"),
            grid_bind,
            scan_bind,
            density_bind,
            sweeps: options.bench_sweeps,
            particle_groups: count.div_ceil(256),
            cell_groups: cells.div_ceil(256),
            validation: Some(Validation {
                seeded,
                grid,
                h,
                mass,
                starts,
                density,
                staging,
            }),
        }
    }

    /// Dispatch order is the data dependency; WebGPU orders storage
    /// writes between dispatches in one pass.
    fn encode(&self, pass: &mut wgpu::ComputePass<'_>) {
        pass.set_bind_group(0, &self.grid_bind, &[]);
        pass.set_pipeline(&self.count);
        pass.dispatch_workgroups(self.particle_groups, 1, 1);
        pass.set_bind_group(0, &self.scan_bind, &[]);
        pass.set_pipeline(&self.scan_blocks);
        pass.dispatch_workgroups(self.cell_groups, 1, 1);
        pass.set_pipeline(&self.scan_sums);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.add_back);
        pass.dispatch_workgroups(self.cell_groups, 1, 1);
        pass.set_bind_group(0, &self.grid_bind, &[]);
        pass.set_pipeline(&self.scatter);
        pass.dispatch_workgroups(self.particle_groups, 1, 1);
        pass.set_bind_group(0, &self.density_bind, &[]);
        pass.set_pipeline(&self.density_sweep);
        for _ in 0..self.sweeps {
            pass.dispatch_workgroups(self.particle_groups, 1, 1);
        }
    }
}

/// Blocks once, on the bench's first frame, never on the steady path:
/// the GPU scan against a CPU scan, and every sixteenth density against
/// a brute-force sum over all particles. Returns the two verdicts.
fn validate_bench(device: &wgpu::Device, queue: &wgpu::Queue, v: &Validation) -> [bool; 2] {
    let read = |buffer: &wgpu::Buffer, words: usize| -> Option<Vec<u32>> {
        let bytes = (words * 4) as u64;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &v.staging, 0, bytes);
        queue.submit(std::iter::once(encoder.finish()));
        let done = Arc::new(AtomicU8::new(0));
        let flag = done.clone();
        v.staging.map_async(wgpu::MapMode::Read, ..bytes, move |r| {
            flag.store(if r.is_ok() { 1 } else { 2 }, Ordering::Release);
        });
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        if done.load(Ordering::Acquire) != 1 {
            return None;
        }
        let words = {
            let mapped = v.staging.get_mapped_range(..bytes).expect("mapped");
            mapped
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect()
        };
        v.staging.unmap();
        Some(words)
    };
    let cells = v.grid.cell_count() as usize;
    let particles = v.seeded.as_chunks::<4>().0;
    let scan = read(&v.starts, cells).is_some_and(|gpu| {
        let mut expect = vec![0u32; cells];
        for p in particles {
            expect[v.grid.cell_of([p[0], p[1], p[2]]) as usize] += 1;
        }
        let mut run = 0u32;
        for e in &mut expect {
            let c = *e;
            *e = run;
            run += c;
        }
        gpu == expect
    });
    let density = read(&v.density, particles.len()).is_some_and(|gpu| {
        particles.iter().enumerate().step_by(16).all(|(i, p)| {
            let rho: f32 = particles
                .iter()
                .map(|q| {
                    let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                    v.mass * sim::kernel((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt(), v.h)
                })
                .sum();
            (f32::from_bits(gpu[i]) - rho).abs() <= 1e-3 * rho
        })
    });
    [scan, density]
}

const SLOT_FREE: u8 = 0;
const SLOT_IN_FLIGHT: u8 = 1;
const SLOT_READY: u8 = 2;

struct StagingSlot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

struct GpuTiming {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    staging: [StagingSlot; 3],
    period_ns: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
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
    /// Neighbours dropped past the list cap, cumulative; nonzero means
    /// the solver skipped pairs and the cap in sim_solve.wgsl is short.
    pub neighbour_overflow: u32,
    pub substeps: u32,
    pub idle_frames: u64,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    interval_us: Ring,
    acquire_us: Ring,
    encode_us: Ring,
    gpu_us: Ring,
    gpu_timing: Option<GpuTiming>,
    force_filter: ForceFilter,
    rotation: RotationTracker,
    fingers: Fingers,
    ramp: Ramp,
    idle: IdleGate,
    idle_frames: u64,
    mode: Mode,
    look: Look,
    frames: u64,
    last_frame_ms: f64,
}

impl Renderer {
    /// The shell makes the surface; wgpu types cross this boundary,
    /// platform types do not.
    pub fn new(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        options: RenderOptions,
    ) -> Result<Renderer, String> {
        let adapter = ready(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let (device, queue) = ready(adapter.request_device(&wgpu::DeviceDescriptor {
            // IMMEDIATES and SUBGROUP are unconditional: wgpu grants
            // both on every Metal 3 GPU (A13 and later; the iOS 17 floor
            // also admits the A12, which fails here), and the M3 record
            // forbids a fallback branch no run reaches.
            required_features: wgpu::Features::IMMEDIATES
                | wgpu::Features::SUBGROUP
                | if timestamps {
                    wgpu::Features::TIMESTAMP_QUERY
                } else {
                    wgpu::Features::empty()
                },
            // WebGPU's default limits overshoot small adapters (the
            // simulator offers 15 inter-stage variables, the default
            // asks 16), so start from downlevel and raise what the code
            // binds: the solve layout holds eighteen storage buffers.
            required_limits: wgpu::Limits {
                max_storage_buffers_per_shader_stage: 21,
                max_immediate_size: sim::STEP_BYTES as u32,
                ..wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
            },
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;

        let caps = surface.get_capabilities(&adapter);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: caps.formats[0],
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        // The shaders write linear light, so the format says whether
        // the hardware encodes on write; set_look linearises to match.
        eprintln!("surface: {:?}", config.format);

        let extent = [
            width as f32 * 0.5 * METRES_PER_PIXEL,
            height as f32 * 0.5 * METRES_PER_PIXEL,
        ];
        let mode = if options.bench_sweeps > 0 {
            Mode::Bench(Box::new(Bench::new(&device, extent, &options)))
        } else if options.sim_substeps > 0 {
            // FLUID_SPACING pins the spacing for a measurement run;
            // zero hands the choice to the particle scale.
            let spacing = if options.bench_spacing > 0.0 {
                options
                    .bench_spacing
                    .clamp(0.4 * SIM_SPACING, 4.0 * SIM_SPACING)
            } else {
                spacing_for(options.particle_scale, extent)
            };
            Mode::Sim(Box::new(Sim::new(
                &device,
                config.format,
                extent,
                options.sim_substeps,
                spacing,
                options.tracers,
                [config.width, config.height],
            )))
        } else {
            Mode::Demo(Box::new(Particles::new(
                &device,
                config.format,
                options.particle_count.max(1),
                options.sprite_radius,
                extent,
            )))
        };

        let gpu_timing = timestamps.then(|| GpuTiming {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: None,
                ty: wgpu::QueryType::Timestamp,
                count: 4,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 32,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            staging: std::array::from_fn(|_| StagingSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 32,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(SLOT_FREE)),
            }),
            period_ns: queue.get_timestamp_period(),
        });

        Ok(Renderer {
            device,
            queue,
            surface,
            config,
            interval_us: Ring::new(),
            acquire_us: Ring::new(),
            encode_us: Ring::new(),
            gpu_us: Ring::new(),
            gpu_timing,
            force_filter: ForceFilter::new(),
            rotation: RotationTracker::new(),
            fingers: Fingers::new(),
            ramp: Ramp::new(),
            idle: IdleGate::new(),
            idle_frames: 0,
            mode,
            look: Look::Glass,
            frames: 0,
            last_frame_ms: 0.0,
        })
    }

    /// Reseeds the fluid at `scale` times the 1x count (spacing_for).
    /// A rebuild off the frame path: the sim's buffers and pipelines
    /// are made again, and the idle gate restarts so the fresh lattice
    /// on a still phone falls and settles before it may sleep.
    pub fn set_particles(&mut self, scale: f32) {
        let Mode::Sim(s) = &self.mode else {
            return;
        };
        let spacing = spacing_for(scale, s.extent);
        if spacing == s.spacing {
            return;
        }
        let (extent, cap, tracers) = (s.extent, s.max_substeps, s.tracer_count);
        let started = std::time::Instant::now();
        self.mode = Mode::Sim(Box::new(Sim::new(
            &self.device,
            self.config.format,
            extent,
            cap,
            spacing,
            tracers,
            [self.config.width, self.config.height],
        )));
        self.idle = IdleGate::new();
        eprintln!(
            "sim: rebuilt in {:.0} ms",
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }

    /// The count `set_particles(scale)` would seed; zero outside the sim.
    pub fn particles_at(&self, scale: f32) -> u32 {
        match &self.mode {
            Mode::Sim(s) => sim::lattice_dims(spacing_for(scale, s.extent), s.extent, 0.5)
                .iter()
                .product(),
            _ => 0,
        }
    }

    /// Where one finger presses, in normalised drawable coordinates: x
    /// runs 0 to 1 left to right, y 0 to 1 top to bottom, the shell's
    /// own convention. `down` false lifts it. `slot` is the shell's
    /// name for that finger, held for as long as it stays on the
    /// glass, and ignored past `touch_slots`. Every finger down drags
    /// the water it moves through, and any finger down holds the idle
    /// gate awake.
    pub fn touch(&mut self, slot: u32, x: f32, y: f32, down: bool) {
        if let Some(at) = self.fingers.at.get_mut(slot as usize) {
            *at = down.then_some([x, y]);
        }
    }

    /// The idle gate restarts: a sleeping sim presents nothing, so a
    /// look changed on a still phone would not reach the screen.
    pub fn set_look(&mut self, look: Look) {
        self.look = look;
        self.idle = IdleGate::new();
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        if let Mode::Demo(p) = &mut self.mode {
            p.extent = [
                width as f32 * 0.5 * METRES_PER_PIXEL,
                height as f32 * 0.5 * METRES_PER_PIXEL,
            ];
        }
        if let Mode::Sim(s) = &mut self.mode {
            s.resize(&self.device, [width, height]);
        }
        self.surface.configure(&self.device, &self.config);
    }

    /// `now_ms` is `CADisplayLink.timestamp` in milliseconds; only
    /// differences are taken. Returns false when the settled sim slept
    /// the frame: nothing was encoded or presented, and the shell may
    /// drop its tick rate until the next true.
    pub fn frame(&mut self, sample: MotionSample, now_ms: f64) -> bool {
        // A gap after a pause is not a frame interval; half a second cuts
        // off resumes without hiding real hitches.
        let interval_us = ((now_ms - self.last_frame_ms) * 1_000.0) as f32;
        let was_asleep = self.idle.sleeping();
        let (force, dev) = self.force_filter.apply(sample.body_force());
        let (omega, domega) = self
            .rotation
            .apply(sample.rotation_rate, interval_us / 1_000_000.0);
        let extent = match &self.mode {
            Mode::Sim(s) => s.extent,
            _ => [1.0; 2],
        };
        let flat = !matches!(self.look, Look::Glass);
        // The particle view draws the solver's own particles, so it
        // needs no thickness field: the splat, the blur and the
        // surface pass go unencoded.
        let field = !matches!(self.look, Look::Particles(_));
        // The strands are the glass look's dye; neither flat look
        // shows them.
        let strands = !flat && matches!(&self.mode, Mode::Sim(s) if s.tracer_count > 0);
        if let Mode::Sim(s) = &self.mode
            && self
                .idle
                .asleep(self.fingers.any_down(), force, dev, omega, s.stats[6])
        {
            // The clock still advances, so the wake frame steps one
            // tick, not the whole nap.
            self.last_frame_ms = now_ms;
            self.idle_frames += 1;
            return false;
        }
        // Idle ticks run at the shell's nap rate; their intervals are
        // naps, not frame times.
        if self.frames > 0 && !was_asleep && interval_us < 500_000.0 {
            self.interval_us.push(interval_us);
        }
        self.last_frame_ms = now_ms;
        let dt = if self.frames == 0 {
            0.0
        } else {
            (interval_us / 1_000_000.0).clamp(0.0, MAX_DT)
        };
        let touches = if matches!(self.mode, Mode::Sim(_)) {
            self.fingers.step(extent, dt)
        } else {
            sim::Touches::default()
        };
        // The ramp chases the frame's own two ends, so it needs the
        // frame's dt and the readback the gate above kept from
        // advancing.
        let ends = match (&self.mode, self.look) {
            (
                Mode::Sim(s),
                Look::Flat(Paint::Ramp { lens, .. }) | Look::Particles(Paint::Ramp { lens, .. }),
            ) => self.ramp.follow(lens, &s.stats, s.spacing, dt),
            _ => None,
        };
        let paint = Painted::new(self.look, ends);
        let optics = match &self.mode {
            Mode::Sim(s) => pack_optics(force, extent, s.field_settled, paint.low, paint.high),
            _ => [0; 80],
        };
        // The direction wheel is the one lens the surface cannot read
        // out of the thickness field.
        let wheel = field && paint.high[3] > 1.0;
        if let Mode::Sim(s) = &mut self.mode {
            // CFL: dt <= 0.4 d / v_max, from the one-frame-stale v_max
            // the stats readback drained. The GPU clamp enforces the dt
            // actually encoded.
            s.substeps_used = if dt > 0.0 {
                ((dt * s.stats[6] / (0.4 * s.spacing)).ceil() as u32)
                    .max(substep_floor(dt))
                    .min(s.max_substeps)
            } else {
                0
            };
            // A quarter-step chase, so keep cannot pop between frames
            // when v_max hovers at the fade edge.
            s.field_keep += (field_keep_target(s.stats[6]) - s.field_keep) * 0.25;
            s.frame_seed = s.frame_seed.wrapping_add(1);
        }

        // The drawable acquire blocks on swapchain back-pressure; timed
        // apart from the encode, or the block masquerades as CPU work
        // (it was ~97% of the old combined number).
        let acquire_started = std::time::Instant::now();

        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return true;
            }
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => {
                self.surface.configure(&self.device, &self.config);
                return true;
            }
        };
        self.acquire_us
            .push(acquire_started.elapsed().as_secs_f32() * 1_000_000.0);
        let started = std::time::Instant::now();

        self.drain_ready_slots();
        let slot = self.gpu_timing.as_ref().and_then(|t| {
            t.staging
                .iter()
                .position(|s| s.state.load(Ordering::Acquire) == SLOT_FREE)
        });

        if let Mode::Demo(p) = &self.mode {
            self.queue.write_buffer(
                &p.params,
                0,
                &particles::pack_params([force[0], force[1]], dt, p.radius, p.extent, p.count),
            );
        }

        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            // Each pass carries its own timestamp pair and gpu_us sums
            // the spans: one span stretched across both passes read zero
            // on the reference device (2026-08-30).
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: self.gpu_timing.as_ref().and_then(|t| {
                    slot.map(|_| wgpu::ComputePassTimestampWrites {
                        query_set: &t.queries,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    })
                }),
            });
            match &self.mode {
                Mode::Demo(p) => {
                    pass.set_pipeline(&p.integrate);
                    pass.set_bind_group(0, &p.integrate_bind, &[]);
                    pass.dispatch_workgroups(p.count.div_ceil(64), 1, 1);
                }
                Mode::Bench(b) => b.encode(&mut pass),
                // A dt of zero (the first frame, a resume) encodes no
                // solve: kappa divides by dt squared, and NaN from a
                // zero dt poisons every buffer it touches.
                Mode::Sim(s) if dt > 0.0 => {
                    let n = s.substeps_used;
                    let dt_sub = dt / n as f32;
                    let v_clamp = 0.4 * s.spacing / dt_sub;
                    let step = sim::pack_step(force, omega, domega, dt_sub, v_clamp, 0, touches);
                    let particles = s.count.div_ceil(256);
                    let wide = (s.count * SWEEP_LANES).div_ceil(256);
                    for _ in 0..n {
                        pass.set_bind_group(0, &s.grid_bind, &[]);
                        pass.set_pipeline(&s.count_cells);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_bind_group(0, &s.scan_bind, &[]);
                        pass.set_pipeline(&s.scan_single);
                        pass.dispatch_workgroups(1, 1, 1);
                        pass.set_bind_group(0, &s.grid_bind, &[]);
                        pass.set_pipeline(&s.scatter);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_bind_group(0, &s.solve_bind, &[]);
                        pass.set_pipeline(&s.density_div);
                        pass.set_immediates(0, &step);
                        pass.dispatch_workgroups(wide, 1, 1);
                        pass.set_pipeline(&s.div_apply);
                        pass.dispatch_workgroups(wide, 1, 1);
                        pass.set_pipeline(&s.forces_eval);
                        pass.dispatch_workgroups(wide, 1, 1);
                        pass.set_pipeline(&s.forces_den_apply);
                        pass.dispatch_workgroups(wide, 1, 1);
                        for _ in 0..refine_passes(dt_sub) {
                            pass.set_pipeline(&s.den_kappa);
                            pass.dispatch_workgroups(wide, 1, 1);
                            pass.set_pipeline(&s.den_apply);
                            pass.dispatch_workgroups(wide, 1, 1);
                        }
                        pass.set_pipeline(&s.integrate);
                        pass.dispatch_workgroups(particles, 1, 1);
                    }
                    pass.set_pipeline(&s.reduce_stats);
                    pass.dispatch_workgroups(1, 1, 1);
                    if strands {
                        // The visual layer advects once a frame on the
                        // solved end-of-frame field, with the frame dt.
                        pass.set_bind_group(0, &s.tracer_bind, &[]);
                        pass.set_pipeline(&s.splat);
                        pass.set_immediates(
                            0,
                            &sim::pack_step(
                                force,
                                [0.0; 3],
                                [0.0; 3],
                                dt,
                                0.0,
                                s.frame_seed,
                                sim::Touches::default(),
                            ),
                        );
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_pipeline(&s.resolve);
                        pass.dispatch_workgroups(s.cell_groups, 1, 1);
                        pass.set_pipeline(&s.advect);
                        pass.dispatch_workgroups(s.tracer_count.div_ceil(256), 1, 1);
                    }
                }
                Mode::Sim(_) => {}
            }
        }
        if let Mode::Sim(s) = &self.mode
            && field
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &s.field,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                ..Default::default()
            });
            // After a frame that skipped the field, the first field
            // frame decays the stale one to nothing and splats the
            // whole weight: the same steady state, no ghost.
            let keep = if s.field_live {
                f64::from(s.field_keep)
            } else {
                0.0
            };
            let splat = 1.0 - keep;
            pass.set_pipeline(&s.decay);
            pass.set_blend_constant(wgpu::Color {
                r: keep,
                g: keep,
                b: keep,
                a: keep,
            });
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&s.body);
            pass.set_immediates(
                0,
                &pack_paint(paint.low, paint.high, paint.range, paint.lens, 0.0),
            );
            pass.set_blend_constant(wgpu::Color {
                r: splat,
                g: splat,
                b: splat,
                a: splat,
            });
            pass.set_bind_group(0, &s.sprite_bind, &[]);
            pass.draw(0..4, 0..s.count);
        }
        if let Mode::Sim(s) = &self.mode
            && wheel
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &s.flow,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                ..Default::default()
            });
            let keep = if s.flow_live {
                f64::from(s.field_keep)
            } else {
                0.0
            };
            let splat = 1.0 - keep;
            pass.set_pipeline(&s.decay);
            pass.set_blend_constant(wgpu::Color {
                r: keep,
                g: keep,
                b: keep,
                a: keep,
            });
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&s.body_flow);
            pass.set_immediates(
                0,
                &pack_paint(paint.low, paint.high, paint.range, paint.lens, 0.0),
            );
            pass.set_blend_constant(wgpu::Color {
                r: splat,
                g: splat,
                b: splat,
                a: splat,
            });
            pass.set_bind_group(0, &s.sprite_bind, &[]);
            pass.draw(0..4, 0..s.count);
        }
        if let Mode::Sim(s) = &self.mode
            && field
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&s.filter);
            pass.set_bind_group(0, &s.filter_bind, &[]);
            pass.set_immediates(0, &optics);
            pass.dispatch_workgroups(s.filter_groups[0], s.filter_groups[1], 1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if field {
                            BACKDROP
                        } else {
                            wgpu::Color::BLACK
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.gpu_timing.as_ref().and_then(|t| {
                    slot.map(|_| wgpu::RenderPassTimestampWrites {
                        query_set: &t.queries,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    })
                }),
                occlusion_query_set: None,
                ..Default::default()
            });
            match &self.mode {
                Mode::Demo(p) => {
                    pass.set_pipeline(&p.sprites);
                    pass.set_bind_group(0, &p.sprite_bind, &[]);
                    pass.draw(0..4, 0..p.count);
                }
                Mode::Sim(s) if !field => {
                    // Clip space spans the extent, so a pixel is that
                    // much of it over half the drawable's width.
                    let metres_per_px = 2.0 * s.extent[0] / self.config.width as f32;
                    pass.set_pipeline(&s.discs);
                    pass.set_immediates(
                        0,
                        &pack_paint(
                            paint.low,
                            paint.high,
                            paint.range,
                            paint.lens,
                            MIN_DISC_PX * metres_per_px,
                        ),
                    );
                    pass.set_bind_group(0, &s.sprite_bind, &[]);
                    pass.draw(0..4, 0..s.count);
                }
                Mode::Sim(s) => {
                    pass.set_pipeline(&s.fill);
                    pass.set_immediates(0, &optics);
                    pass.set_bind_group(0, &s.fill_bind, &[]);
                    pass.draw(0..3, 0..1);
                    if strands {
                        pass.set_pipeline(&s.points);
                        pass.set_bind_group(0, &s.tracer_draw_bind, &[]);
                        pass.draw(0..s.tracer_count, 0..1);
                    // The flat surface is the whole picture: two
                    // colours, and nothing drawn over them.
                    } else if !flat {
                        pass.set_pipeline(&s.sprites);
                        pass.set_bind_group(0, &s.sprite_bind, &[]);
                        pass.draw(0..4, 0..s.count);
                    }
                }
                Mode::Bench(_) => {}
            }
        }
        if let (Some(t), Some(slot)) = (self.gpu_timing.as_ref(), slot) {
            encoder.resolve_query_set(&t.queries, 0..4, &t.resolve, 0);
            encoder.copy_buffer_to_buffer(&t.resolve, 0, &t.staging[slot].buffer, 0, 32);
        }
        let sim_slot = if let Mode::Sim(s) = &self.mode {
            let free = s
                .stats_staging
                .iter()
                .position(|x| x.state.load(Ordering::Acquire) == SLOT_FREE);
            if let Some(i) = free {
                encoder.copy_buffer_to_buffer(
                    &s.stats_src,
                    0,
                    &s.stats_staging[i].buffer,
                    0,
                    STATS_BYTES,
                );
            }
            free
        } else {
            None
        };
        if let Mode::Sim(s) = &mut self.mode {
            s.field_live = field;
            s.flow_live = wheel;
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(texture);
        if let (Some(t), Some(slot)) = (self.gpu_timing.as_ref(), slot) {
            let slot = &t.staging[slot];
            slot.state.store(SLOT_IN_FLIGHT, Ordering::Release);
            let state = slot.state.clone();
            slot.buffer
                .map_async(wgpu::MapMode::Read, .., move |result| {
                    state.store(
                        if result.is_ok() {
                            SLOT_READY
                        } else {
                            SLOT_FREE
                        },
                        Ordering::Release,
                    );
                });
        }
        if let (Mode::Sim(s), Some(i)) = (&self.mode, sim_slot) {
            let slot = &s.stats_staging[i];
            slot.state.store(SLOT_IN_FLIGHT, Ordering::Release);
            let state = slot.state.clone();
            slot.buffer
                .map_async(wgpu::MapMode::Read, .., move |result| {
                    state.store(
                        if result.is_ok() {
                            SLOT_READY
                        } else {
                            SLOT_FREE
                        },
                        Ordering::Release,
                    );
                });
        }
        if self.gpu_timing.is_some() || matches!(self.mode, Mode::Sim(_)) {
            let _ = self.device.poll(wgpu::PollType::Poll);
        }

        self.encode_us
            .push(started.elapsed().as_secs_f32() * 1_000_000.0);
        self.frames += 1;

        if let Mode::Bench(b) = &mut self.mode
            && let Some(v) = b.validation.take()
        {
            let verdict = |ok: bool| if ok { "PASS" } else { "FAIL" };
            let [scan, density] = validate_bench(&self.device, &self.queue, &v);
            eprintln!(
                "bench validation: scan {}, density {}",
                verdict(scan),
                verdict(density)
            );
        }
        true
    }

    fn drain_ready_slots(&mut self) {
        if let Mode::Sim(s) = &mut self.mode {
            for slot in &s.stats_staging {
                if slot.state.load(Ordering::Acquire) != SLOT_READY {
                    continue;
                }
                {
                    let bytes = slot
                        .buffer
                        .get_mapped_range(..)
                        .expect("mapped by the map_async callback");
                    for (v, chunk) in s.stats.iter_mut().zip(bytes.as_chunks::<4>().0) {
                        *v = f32::from_le_bytes(*chunk);
                    }
                }
                slot.buffer.unmap();
                slot.state.store(SLOT_FREE, Ordering::Release);
            }
        }
        let Some(t) = self.gpu_timing.as_ref() else {
            return;
        };
        for slot in &t.staging {
            if slot.state.load(Ordering::Acquire) != SLOT_READY {
                continue;
            }
            let mut stamps = [0u64; 4];
            {
                let bytes = slot
                    .buffer
                    .get_mapped_range(..)
                    .expect("mapped by the map_async callback");
                let (chunks, _) = bytes.as_chunks::<8>();
                for (stamp, chunk) in stamps.iter_mut().zip(chunks) {
                    // Timestamps land in device byte order; every target here
                    // is little-endian.
                    *stamp = u64::from_le_bytes(*chunk);
                }
            }
            slot.buffer.unmap();
            slot.state.store(SLOT_FREE, Ordering::Release);
            let delta_ns = (stamps[1].saturating_sub(stamps[0])
                + stamps[3].saturating_sub(stamps[2])) as f32
                * t.period_ns;
            self.gpu_us.push(delta_ns / 1_000.0);
        }
    }

    /// Off the frame path; the shells call this about once per second.
    pub fn stats(&self) -> RenderStats {
        let (f, substeps) = match &self.mode {
            Mode::Sim(s) => (s.stats, s.substeps_used),
            _ => ([0.0; STATS], 0),
        };
        RenderStats {
            compression_avg: f[0],
            compression_max: f[1],
            density_min: f[2],
            density_max: f[3],
            pressure_min: f[4],
            pressure_max: f[5],
            v_max: f[6],
            temperature_min: f[7],
            temperature_max: f[8],
            clamp_count: f[9] as u32,
            neighbour_overflow: f[10] as u32,
            substeps,
            idle_frames: self.idle_frames,
            frames: self.frames,
            interval_p50_us: self.interval_us.percentile(0.5),
            interval_p99_us: self.interval_us.percentile(0.99),
            interval_max_us: self.interval_us.max(),
            acquire_p50_us: self.acquire_us.percentile(0.5),
            acquire_p99_us: self.acquire_us.percentile(0.99),
            encode_p50_us: self.encode_us.percentile(0.5),
            encode_p99_us: self.encode_us.percentile(0.99),
            gpu_p50_us: self.gpu_us.percentile(0.5),
            gpu_p99_us: self.gpu_us.percentile(0.99),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_gate_sleeps_when_still_and_wakes_on_shake_or_tilt() {
        let g = 9.81;
        let still = [0.0f32; 3];
        let upright = [0.0, -g, 0.0];
        let mut gate = IdleGate::new();
        for _ in 0..IdleGate::STILL_FRAMES - 1 {
            assert!(!gate.asleep(false, upright, 0.1, still, 0.03));
        }
        assert!(gate.asleep(false, upright, 0.1, still, 0.03));
        // Noise peaks sit between the sleep and wake thresholds: no
        // false wake.
        assert!(gate.asleep(false, upright, 0.8, still, 0.03));
        // A shake crosses the deviation test in one tick.
        assert!(!gate.asleep(false, upright, 2.0, still, 0.03));
        for _ in 0..IdleGate::STILL_FRAMES {
            gate.asleep(false, upright, 0.1, still, 0.03);
        }
        assert!(gate.asleep(false, upright, 0.1, still, 0.03));
        // A settled 2.9-degree tilt is far past the 1.5-degree wake
        // angle even though its deviation never crossed anything.
        assert!(!gate.asleep(false, [g * 0.05, -g, 0.0], 0.1, still, 0.03));
    }

    // The gyro-only pose: a flat phone spun about its normal holds
    // gravity fixed in the box frame, so rotation is the one signal
    // that can move the water - the gate must see it.
    #[test]
    fn idle_gate_wakes_on_pure_rotation() {
        let flat = [0.0, 0.0, -9.81];
        let mut gate = IdleGate::new();
        for _ in 0..IdleGate::STILL_FRAMES {
            gate.asleep(false, flat, 0.1, [0.0; 3], 0.03);
        }
        assert!(
            gate.asleep(false, flat, 0.1, [0.0, 0.0, 0.01], 0.03),
            "gyro noise"
        );
        assert!(
            !gate.asleep(false, flat, 0.1, [0.0, 0.0, 0.5], 0.03),
            "a real turn"
        );
        // And a spin blocks falling asleep in the first place.
        let mut spun = IdleGate::new();
        for _ in 0..2 * IdleGate::STILL_FRAMES {
            assert!(!spun.asleep(false, flat, 0.1, [0.0, 0.0, 0.5], 0.03));
        }
    }

    // The shell reports a point on its own drawable and nothing else;
    // every metre below is the core's (D6).
    #[test]
    fn the_fingers_map_drawable_points_into_the_box() {
        let extent = [0.1, 0.2];
        let mut fingers = Fingers::new();
        fingers.at[0] = Some([1.0, 0.0]);
        let touches = fingers.step(extent, 0.0);
        assert_eq!(touches.count, 1);
        assert_eq!(
            touches.each[0].at,
            [0.1, 0.2],
            "the drawable's top right is the box's"
        );
        assert_eq!(
            touches.each[0].velocity, [0.0; 2],
            "one sample makes no speed"
        );
        assert!(touches.radius > 0.0);
        fingers.at[0] = None;
        assert_eq!(fingers.step(extent, 1.0 / 120.0).count, 0, "lifted");
    }

    // The slot is the shell's name for one finger. A finger that lifts
    // must not hand its speed to whichever finger the solver reads in
    // its place: the live fingers pack forward for the shader, the
    // state behind them does not move.
    #[test]
    fn a_lifted_finger_leaves_the_others_where_they_were() {
        let extent = [0.1, 0.2];
        let dt = 1.0 / 120.0;
        let mut fingers = Fingers::new();
        let mut velocity = [0.0; 2];
        for f in 0..60 {
            // Slot 2 walks a hundredth of the drawable's width a frame:
            // a fifth of a millimetre of box in 1/120 s, 0.24 m/s right.
            fingers.at[0] = Some([0.9, 0.9]);
            fingers.at[2] = Some([0.2 + 0.01 * f as f32, 0.5]);
            let touches = fingers.step(extent, dt);
            assert_eq!(touches.count, 2);
            velocity = touches.each[1].velocity;
        }
        assert!((velocity[0] - 0.24).abs() < 0.005, "vx {}", velocity[0]);
        assert_eq!(velocity[1], 0.0);
        // Lift the still finger; the walking one keeps walking. It now
        // reads from slot 0 of the packed array instead of slot 1, and
        // must still be the same finger at the same speed.
        fingers.at[0] = None;
        fingers.at[2] = Some([0.2 + 0.01 * 60.0, 0.5]);
        let touches = fingers.step(extent, dt);
        assert_eq!(touches.count, 1);
        assert!(
            (touches.each[0].velocity[0] - 0.24).abs() < 0.005,
            "the walking finger changed when the still one lifted: {:?}",
            touches.each[0].velocity
        );
    }

    #[test]
    fn percentiles_come_from_the_filled_part_only() {
        let mut ring = Ring::new();
        assert_eq!(ring.percentile(0.5), 0.0);
        for v in [30.0, 10.0, 20.0] {
            ring.push(v);
        }
        assert_eq!(ring.percentile(0.0), 10.0);
        assert_eq!(ring.percentile(0.5), 20.0);
        assert_eq!(ring.percentile(1.0), 30.0);
        assert_eq!(ring.max(), 30.0);
    }

    #[test]
    fn the_ring_overwrites_its_oldest_sample() {
        let mut ring = Ring::new();
        for i in 0..(RING + 1) {
            ring.push(i as f32);
        }
        assert_eq!(ring.percentile(0.0), 1.0);
        assert_eq!(ring.percentile(1.0), RING as f32);
    }

    #[test]
    fn optics_immediates_land_at_the_shader_offsets() {
        let raw = pack_optics(
            [0.0, -9.81, 0.0],
            [4.0, 5.0],
            FIELD_SETTLED,
            [0.25, 0.5, 0.75, 1.0],
            [0.1, 0.2, 0.3, 1.0],
        );
        let f = |i: usize| f32::from_le_bytes(raw[i..i + 4].try_into().unwrap());
        assert_eq!([f(0), f(4), f(8)], [0.0, 1.0, 0.0], "up");
        assert_eq!(f(12), FIELD_SETTLED);
        assert_eq!([f(16), f(20)], [4.0, 5.0], "extent");
        assert_eq!(
            [f(48), f(52), f(56), f(60)],
            [0.25, 0.5, 0.75, 1.0],
            "the flat look"
        );
        assert_eq!(
            [f(64), f(68), f(72), f(76)],
            [0.1, 0.2, 0.3, 1.0],
            "the ramp's high colour"
        );
        assert_eq!(f(24), sim::SLAB_DEPTH);
        // Upright: up.z = 0, no fade, so the gain is SUN scaled by
        // Schlick at the half vector's z.
        let h = [f(32), f(36), f(40)];
        assert!(
            (h[0] * h[0] + h[1] * h[1] + h[2] * h[2] - 1.0).abs() < 1e-5,
            "h unit"
        );
        let schlick = GLINT_F0 + (1.0 - GLINT_F0) * (1.0 - h[2]).powi(5);
        assert!((f(28) - SUN * schlick).abs() < 1e-4, "gain {}", f(28));
        // Face up fades the sun out; face down degenerates h to zero
        // and the gain with it — neither may go NaN.
        let flat = pack_optics(
            [0.0, 0.0, -9.81],
            [4.0, 5.0],
            FIELD_SETTLED,
            [0.0; 4],
            [0.0; 4],
        );
        assert_eq!(
            f32::from_le_bytes(flat[28..32].try_into().unwrap()),
            0.0,
            "face-up fade"
        );
        let down = pack_optics(
            [0.0, 0.0, 9.81],
            [4.0, 5.0],
            FIELD_SETTLED,
            [0.0; 4],
            [0.0; 4],
        );
        for chunk in down.as_chunks::<4>().0 {
            assert!(f32::from_le_bytes(*chunk).is_finite());
        }
    }

    // Hot pink's bytes through sRGB's curve: the ends are fixed points,
    // the middle darkens, and the glass is all zeros.
    #[test]
    fn the_flat_colour_is_linearised() {
        let hot_pink = [1.0, 105.0 / 255.0, 180.0 / 255.0];
        let solid = |look| Painted::new(look, None);
        let [r, g, b, w] = solid(Look::Flat(Paint::Solid(hot_pink))).low;
        assert_eq!((r, w), (1.0, 1.0));
        // The discs take their colour from the same word.
        assert_eq!(
            solid(Look::Particles(Paint::Solid(hot_pink))).low,
            [r, g, b, w]
        );
        assert!((g - 0.1413).abs() < 1e-3, "g {g}");
        assert!((b - 0.4564).abs() < 1e-3, "b {b}");
        assert_eq!(linear(0.0), 0.0);
        assert!(
            (linear(0.04045) - 0.04045 / 12.92).abs() < 1e-6,
            "the joint"
        );
        assert_eq!(solid(Look::Glass).low, [0.0; 4]);
    }

    // The menu's four scales on the reference screen: within 5% of
    // their multiples of the 1x count, and 1x is the shipped spacing.
    #[test]
    fn the_ladder_seeds_near_its_scales() {
        let extent = [
            1284.0 * 0.5 * METRES_PER_PIXEL,
            2778.0 * 0.5 * METRES_PER_PIXEL,
        ];
        let count = |scale| {
            sim::lattice_dims(spacing_for(scale, extent), extent, 0.5)
                .iter()
                .product::<u32>()
        };
        assert_eq!(spacing_for(1.0, extent), SIM_SPACING);
        assert_eq!(count(1.0), 1620);
        for scale in [0.25, 1.0, 4.0, 16.0] {
            let spacing = spacing_for(scale, extent);
            let cells = sim::Grid::new(extent, 2.4 * spacing).cell_count();
            eprintln!(
                "{scale}x: spacing {spacing} m, {} particles, {cells} cells",
                count(scale)
            );
            let ratio = count(scale) as f32 / (scale * 1620.0);
            assert!(
                (0.95..1.05).contains(&ratio),
                "{scale}x seeds {}",
                count(scale)
            );
        }
    }

    /// The box every headless test builds, and the box read_tracers must
    /// unpack the quantised positions against.
    const TEST_EXTENT: [f32; 2] = [crate::WORLD_SCALE * 0.0357, crate::WORLD_SCALE * 0.0774];

    fn headless_sim() -> Option<(wgpu::Device, wgpu::Queue, Sim)> {
        let (device, queue) = headless_device()?;
        let sim = Sim::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            TEST_EXTENT,
            7,
            SIM_SPACING,
            4096,
            [128, 256],
        );
        Some((device, queue, sim))
    }

    fn read_stats(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sim: &Sim,
        substeps: u32,
        frames: u32,
        gravity: [f32; 3],
        touches: sim::Touches,
    ) -> [f32; STATS] {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stats staging"),
            size: STATS_BYTES,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 120 Hz frames split into substeps like the phone would.
        let dt = 1.0 / 120.0 / substeps.max(1) as f32;
        let v_clamp = if substeps == 0 {
            f32::MAX
        } else {
            0.4 * SIM_SPACING / dt
        };
        let step = sim::pack_step(
            gravity,
            [0.0; 3],
            [0.0; 3],
            if substeps == 0 { 0.0 } else { dt },
            v_clamp,
            0,
            touches,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            let particles = sim.count.div_ceil(256);
            let wide = (sim.count * SWEEP_LANES).div_ceil(256);
            for f in 0..frames {
                for _ in 0..substeps.max(1) {
                    pass.set_bind_group(0, &sim.grid_bind, &[]);
                    pass.set_pipeline(&sim.count_cells);
                    pass.dispatch_workgroups(particles, 1, 1);
                    pass.set_bind_group(0, &sim.scan_bind, &[]);
                    pass.set_pipeline(&sim.scan_single);
                    pass.dispatch_workgroups(1, 1, 1);
                    pass.set_bind_group(0, &sim.grid_bind, &[]);
                    pass.set_pipeline(&sim.scatter);
                    pass.dispatch_workgroups(particles, 1, 1);
                    pass.set_bind_group(0, &sim.solve_bind, &[]);
                    pass.set_pipeline(&sim.density_div);
                    pass.set_immediates(0, &step);
                    pass.dispatch_workgroups(wide, 1, 1);
                    if substeps > 0 {
                        pass.set_pipeline(&sim.div_apply);
                        pass.dispatch_workgroups(wide, 1, 1);
                        pass.set_pipeline(&sim.forces_eval);
                        pass.dispatch_workgroups(wide, 1, 1);
                        pass.set_pipeline(&sim.forces_den_apply);
                        pass.dispatch_workgroups(wide, 1, 1);
                        for _ in 0..5 {
                            pass.set_pipeline(&sim.den_kappa);
                            pass.dispatch_workgroups(wide, 1, 1);
                            pass.set_pipeline(&sim.den_apply);
                            pass.dispatch_workgroups(wide, 1, 1);
                        }
                        pass.set_pipeline(&sim.integrate);
                        pass.dispatch_workgroups(particles, 1, 1);
                    }
                }
                if sim.tracer_count > 0 {
                    let frame_dt = if substeps == 0 { 0.0 } else { 1.0 / 120.0 };
                    pass.set_bind_group(0, &sim.tracer_bind, &[]);
                    pass.set_pipeline(&sim.splat);
                    pass.set_immediates(
                        0,
                        &sim::pack_step(
                            gravity,
                            [0.0; 3],
                            [0.0; 3],
                            frame_dt,
                            0.0,
                            f,
                            sim::Touches::default(),
                        ),
                    );
                    pass.dispatch_workgroups(particles, 1, 1);
                    pass.set_pipeline(&sim.resolve);
                    pass.dispatch_workgroups(sim.cell_groups, 1, 1);
                    pass.set_pipeline(&sim.advect);
                    pass.dispatch_workgroups(sim.tracer_count.div_ceil(256), 1, 1);
                }
            }
            pass.set_bind_group(0, &sim.solve_bind, &[]);
            pass.set_pipeline(&sim.reduce_stats);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&sim.stats_src, 0, &staging, 0, STATS_BYTES);
        queue.submit(std::iter::once(encoder.finish()));
        staging.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        let bytes = staging.get_mapped_range(..).expect("mapped");
        let mut out = [0.0f32; STATS];
        for (v, chunk) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *v = f32::from_le_bytes(*chunk);
        }
        out
    }

    // Compiles every sim shader and pipeline on this machine's GPU, so a
    // WGSL error fails here instead of as a crash loop on the phone.
    #[test]
    fn the_sim_gpu_path_compiles_on_this_machine() {
        let Some((device, _queue, _sim)) = ({
            let scope_device;
            let r = headless_sim();
            match r {
                Some((d, q, s)) => {
                    scope_device = d;
                    Some((scope_device, q, s))
                }
                None => None,
            }
        }) else {
            return;
        };
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
    }

    /// Draws the discs of a settled slab into a square of `side` and
    /// hands back its BGRA pixels. The paint is the caller's, so one
    /// draw serves the solid look and the ramp alike.
    // Jack, 2026-09-02: "a hue-wheel rainbow colouring based on the
    // theta of the velocity". The disc draw carries a hue and a
    // saturation down one interpolant, packed into clip z, and nothing
    // else in the suite would notice if that pair came apart. Two
    // claims: every disc is a pure hue scaled by its speed, which
    // fails the moment the pack leaks one number into the other, and a
    // sloshing pool paints its water around the wheel rather than into
    // one corner of it.
    #[test]
    fn the_wheel_paints_pure_hues_around_the_circle() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        // Tipped hard on its side: the pool runs, and its water ends up
        // going every way at once.
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            60,
            [9.81, -2.0, -0.5],
            sim::Touches::default(),
        );
        let side = 64u32;
        let floor = MIN_DISC_PX * 2.0 * TEST_EXTENT[0] / side as f32;
        // Black for the low colour, so a drawn pixel is the hue itself
        // scaled by speed and nothing is mixed into it.
        let paint = pack_paint(
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 2.0],
            Lens::Direction.ends(&f),
            Lens::Direction.code(),
            floor,
        );
        let mut sextants = std::collections::BTreeSet::new();
        let mut drawn = 0;
        for px in draw_discs(&device, &queue, &sim, side, paint) {
            let [b, g, r, _] = px;
            let rgb = [r, g, b];
            if rgb == [0; 3] {
                continue;
            }
            drawn += 1;
            // A hue at full saturation always has a channel at zero,
            // and scaling by the speed leaves it there.
            assert!(
                rgb.iter().min() <= Some(&1),
                "a disc drew something that is not a hue: {rgb:?}"
            );
            let arg = |pick: fn(&&u8, &&u8) -> std::cmp::Ordering| {
                rgb.iter()
                    .enumerate()
                    .max_by(|a, b| pick(&a.1, &b.1))
                    .expect("three channels")
                    .0
            };
            sextants.insert((arg(|a, b| a.cmp(b)), arg(|a, b| b.cmp(a))));
        }
        eprintln!(
            "wheel: {drawn} discs drawn over {} sextants",
            sextants.len()
        );
        assert!(drawn > 100, "the wheel drew {drawn} pixels");
        assert!(
            sextants.len() >= 3,
            "a running pool painted {} of the wheel: {sextants:?}",
            sextants.len()
        );
    }

    // The wheel down the other path. The surface cannot take a mean of
    // an angle, so each particle splats its unit heading into a second
    // field and the fill reads the heading of the sum; nothing else in
    // the suite would notice if that field went missing, and a missing
    // one reads as a single colour, not as a crash.
    #[test]
    fn the_wheel_paints_the_flat_surface_around_the_circle() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            60,
            [9.81, -2.0, -0.5],
            sim::Touches::default(),
        );
        let side = 64u32;
        let black = [0.0, 0.0, 0.0, 1.0];
        let optics = pack_optics(
            [9.81, -2.0, -0.5],
            sim.extent,
            sim.field_settled,
            black,
            [0.0, 0.0, 0.0, 2.0],
        );
        let paint = pack_paint(
            black,
            [0.0, 0.0, 0.0, 2.0],
            Lens::Direction.ends(&f),
            Lens::Direction.code(),
            0.0,
        );
        let mut sextants = std::collections::BTreeSet::new();
        let mut water = 0;
        for px in draw_surface(&device, &queue, &sim, side, paint, optics) {
            let [b, g, r, _] = px;
            let rgb = [r, g, b];
            if rgb == [0; 3] {
                continue;
            }
            water += 1;
            assert!(
                rgb.iter().min() <= Some(&1),
                "the surface drew something that is not a hue: {rgb:?}"
            );
            let arg = |pick: fn(&&u8, &&u8) -> std::cmp::Ordering| {
                rgb.iter()
                    .enumerate()
                    .max_by(|a, b| pick(&a.1, &b.1))
                    .expect("three channels")
                    .0
            };
            sextants.insert((arg(|a, b| a.cmp(b)), arg(|a, b| b.cmp(a))));
        }
        eprintln!(
            "flat wheel: {water} water pixels over {} sextants",
            sextants.len()
        );
        assert!(water > 100, "the wheel drew {water} pixels");
        assert!(
            sextants.len() >= 3,
            "a running pool painted {} of the wheel: {sextants:?}",
            sextants.len()
        );
    }

    // The flat look's whole path: one clean splat into the field, the
    // filter, and the fill. `paint`'s high w picks the ramp or the
    // wheel, and the wheel splats its headings into the second field
    // on the way.
    fn draw_surface(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sim: &Sim,
        side: u32,
        paint: [u8; 48],
        optics: [u8; 80],
    ) -> Vec<[u8; 4]> {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flat surface"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flat surface readback"),
            size: u64::from(side * side * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let wheel =
            f32::from_le_bytes(paint[28..32].try_into().expect("the high colour's w")) > 1.0;
        let mut encoder = device.create_command_encoder(&Default::default());
        for (field, pipeline) in [(&sim.field, &sim.body), (&sim.flow, &sim.body_flow)]
            .into_iter()
            .take(if wheel { 2 } else { 1 })
        {
            // One clean splat: the blend constant is one and the field
            // is cleared, so no frame of decay is in the picture.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: field,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_immediates(0, &paint);
            pass.set_blend_constant(wgpu::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            });
            pass.set_bind_group(0, &sim.sprite_bind, &[]);
            pass.draw(0..4, 0..sim.count);
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&sim.filter);
            pass.set_bind_group(0, &sim.filter_bind, &[]);
            pass.set_immediates(0, &optics);
            pass.dispatch_workgroups(sim.filter_groups[0], sim.filter_groups[1], 1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&sim.fill);
            pass.set_immediates(0, &optics);
            pass.set_bind_group(0, &sim.fill_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * 4),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        let out = readback
            .get_mapped_range(..)
            .expect("mapped")
            .as_chunks::<4>()
            .0
            .to_vec();
        readback.unmap();
        out
    }

    fn draw_discs(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sim: &Sim,
        side: u32,
        paint: [u8; 48],
    ) -> Vec<[u8; 4]> {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle view"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle view readback"),
            size: u64::from(side * side * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&sim.discs);
            pass.set_immediates(0, &paint);
            pass.set_bind_group(0, &sim.sprite_bind, &[]);
            pass.draw(0..4, 0..sim.count);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * 4),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        let bytes = readback.get_mapped_range(..).expect("mapped");
        bytes.as_chunks::<4>().0.to_vec()
    }

    // Jack's rule for the flat look, 2026-09-02: two colours on the
    // screen and nothing between. The discs are opaque and write no
    // alpha, so every pixel is the colour or the black behind it.
    #[test]
    fn the_particle_view_draws_two_colours() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        // A settled second, so the discs size on real densities.
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            120,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        let side = 64u32;
        // Magenta: each component lands on a byte exactly, so a blend
        // or a stray write shows as a third colour.
        let floor = MIN_DISC_PX * 2.0 * TEST_EXTENT[0] / side as f32;
        let paint = pack_paint([1.0, 0.0, 1.0, 1.0], [0.0; 4], [0.0, 1.0], 0, floor);
        let mut water = 0;
        for px in draw_discs(&device, &queue, &sim, side, paint) {
            match px[..3] {
                [0, 0, 0] => {}
                [255, 0, 255] => water += 1,
                _ => panic!("a third colour: {px:?}"),
            }
        }
        assert!(water > side * side / 20, "the discs drew {water} pixels");
    }

    // Jack, 2026-09-02: "make the color a gradient ... two colours
    // denoting low->high values." Red to blue across proximity, which
    // is the lens with real spread in a settled slab: the interior is
    // a full neighbourhood, the free surface is not. Every disc must
    // land somewhere on the line between the two colours, and the
    // discs must not all land on the same point of it.
    #[test]
    fn a_ramp_paints_the_discs_between_its_two_colours() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            120,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        let side = 64u32;
        let floor = MIN_DISC_PX * 2.0 * TEST_EXTENT[0] / side as f32;
        let paint = pack_paint(
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            Lens::Proximity.ends(&f),
            Lens::Proximity.code(),
            floor,
        );
        let mut seen = std::collections::BTreeSet::new();
        for px in draw_discs(&device, &queue, &sim, side, paint) {
            if px[..3] == [0, 0, 0] {
                continue;
            }
            let [b, g, r, _] = px;
            assert_eq!(g, 0, "the ramp left its line: {px:?}");
            assert!(
                (i32::from(r) + i32::from(b) - 255).abs() <= 1,
                "the ramp left its line: {px:?}"
            );
            seen.insert(r);
        }
        eprintln!("proximity ramp: {} distinct steps", seen.len());
        assert!(seen.len() >= 3, "the ramp painted one colour: {seen:?}");
    }

    // The disc law of the particle view (sim_sprites.wgsl): a disc
    // holds its full size while its neighbourhood is a body of water,
    // and shrinks to the pixel floor as that neighbourhood empties.
    // This is the measurement the law's two ends rest on. BODY_RHO
    // mirrors the shader's constant; LONE_RHO is the module's.
    #[test]
    fn the_settled_body_keeps_its_discs() {
        const BODY_RHO: f32 = 0.65;
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        let mut rho: Vec<f32> = read_floats(&device, &queue, &sim.density, sim.count)
            .into_iter()
            .map(|r| r / sim::REST_DENSITY)
            .collect();
        rho.sort_unstable_by(f32::total_cmp);
        let at = |q: f32| rho[((rho.len() - 1) as f32 * q) as usize];
        eprintln!(
            "settled rho/rho0: min {:.3} p1 {:.3} p5 {:.3} p50 {:.3} max {:.3}",
            rho[0],
            at(0.01),
            at(0.05),
            at(0.50),
            rho[rho.len() - 1]
        );
        assert!(
            at(0.05) >= BODY_RHO,
            "a settled body would shrink: p5 {:.3}",
            at(0.05)
        );
        // A lone particle carries its own kernel weight and, mid-slab,
        // no wall support: the far end of the law.
        let h = 1.2 * SIM_SPACING;
        let mass = sim::REST_DENSITY * SIM_SPACING.powi(3);
        let lone = mass * sim::kernel(0.0, h) / sim::REST_DENSITY;
        eprintln!("lone particle: {lone:.3} of rho0");
        // The empty end of the disc law, mirrored from
        // sim_sprites.wgsl.
        const LONE_RHO: f32 = 0.25;
        assert!(lone <= LONE_RHO, "a lone drop would not shrink: {lone:.3}");
    }

    fn read_floats(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        count: u32,
    ) -> Vec<f32> {
        read_back(device, queue, buffer, u64::from(count) * 4)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect()
    }

    fn read_vec4(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        count: u32,
    ) -> Vec<[f32; 4]> {
        read_back(device, queue, buffer, u64::from(count) * 16)
            .as_chunks::<16>()
            .0
            .iter()
            .map(|c| std::array::from_fn(|i| f32::from_le_bytes(c.as_chunks::<4>().0[i])))
            .collect()
    }

    fn read_back(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        size: u64,
    ) -> Vec<u8> {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback staging"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
        queue.submit(std::iter::once(encoder.finish()));
        staging.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        let out = staging.get_mapped_range(..).expect("mapped").to_vec();
        staging.unmap();
        out
    }

    const LENSES: [Lens; 5] = [
        Lens::Velocity,
        Lens::Acceleration,
        Lens::Pressure,
        Lens::Proximity,
        Lens::Direction,
    ];

    /// hue_colour in sim_sprites.wgsl, in the sRGB the picker shows;
    /// the test compares colours against each other, never against the
    /// linear-light ones the shader hands the hardware.
    fn hue_rgb(h: f32) -> [f32; 3] {
        std::array::from_fn(|i| {
            let off = [1.0, 2.0 / 3.0, 1.0 / 3.0][i];
            ((h + off).fract() * 6.0 - 3.0).abs().clamp(1.0, 2.0) - 1.0
        })
    }

    // Jack, 2026-09-02: "It shouldn't repel; it should drag the water
    // as if I were putting my finger through it." The two sides of the
    // finger separate the two models. A repulsive finger throws the
    // water on its left to the left; an entraining one carries both
    // sides along with it, so both sides read positive.
    #[test]
    fn a_finger_drags_both_sides_of_the_water_it_crosses() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        // In the settled pool, a fifth of the box up from the floor,
        // moving right at a metre a second for an eighth of a second.
        let at = [0.0, -0.8 * TEST_EXTENT[1]];
        let mut touches = sim::Touches {
            count: 1,
            radius: Fingers::RADIUS,
            ..Default::default()
        };
        touches.each[0] = sim::Touch {
            at,
            velocity: [1.0, 0.0],
        };
        read_stats(&device, &queue, &sim, 7, 15, [0.0, -9.81, -0.5], touches);
        let positions = read_vec4(&device, &queue, &sim.positions, sim.count);
        let velocities = read_vec4(&device, &queue, &sim.velocities, sim.count);
        let mean = |pick: &dyn Fn(&[f32; 4]) -> bool| {
            let picked: Vec<f32> = positions
                .iter()
                .zip(&velocities)
                .filter(|(p, _)| pick(p))
                .map(|(_, v)| v[0])
                .collect();
            (
                picked.iter().sum::<f32>() / picked.len().max(1) as f32,
                picked.len(),
            )
        };
        let reach = |p: &[f32; 4]| (p[0] - at[0]).hypot(p[1] - at[1]);
        let under = |p: &[f32; 4]| reach(p) < Fingers::RADIUS;
        let (left, n_left) = mean(&|p| under(p) && p[0] < at[0]);
        let (right, n_right) = mean(&|p| under(p) && p[0] > at[0]);
        let (inside, n_in) = mean(&under);
        let (outside, n_out) = mean(&|p| !under(p));
        eprintln!(
            "vx: left {left:.3} ({n_left}) right {right:.3} ({n_right}), \
             under {inside:.3} ({n_in}) beyond {outside:.3} ({n_out})"
        );
        assert!(
            n_left > 0 && n_right > 0 && n_out > 0,
            "the split found nothing"
        );
        assert!(left > 0.0, "the finger repelled its left side: {left:.3}");
        assert!(
            right > 0.0,
            "the finger repelled its right side: {right:.3}"
        );
        assert!(inside > 0.3, "the finger barely bit: {inside:.3}");
        // The net has an edge: water the finger never reaches moves
        // only through the pressure that the dragged water builds.
        assert!(
            outside < 0.5 * inside,
            "the finger dragged the whole box: {outside:.3} against {inside:.3}"
        );
    }

    // Jack, 2026-09-02: "Multi-touch should behave the same way for
    // each simultaneous finger." Two fingers pulling opposite ways is
    // the case a single shared drag cannot fake: each neighbourhood
    // must follow its own finger.
    #[test]
    fn two_fingers_drag_their_own_water_their_own_way() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        // Far enough apart that neither disc reaches the other's water.
        let y = -0.8 * TEST_EXTENT[1];
        let split = 1.2 * Fingers::RADIUS;
        let mut touches = sim::Touches {
            count: 2,
            radius: Fingers::RADIUS,
            ..Default::default()
        };
        touches.each[0] = sim::Touch {
            at: [-split, y],
            velocity: [-1.0, 0.0],
        };
        touches.each[1] = sim::Touch {
            at: [split, y],
            velocity: [1.0, 0.0],
        };
        read_stats(&device, &queue, &sim, 7, 15, [0.0, -9.81, -0.5], touches);
        let positions = read_vec4(&device, &queue, &sim.positions, sim.count);
        let velocities = read_vec4(&device, &queue, &sim.velocities, sim.count);
        let under =
            |p: &[f32; 4], at: [f32; 2]| (p[0] - at[0]).hypot(p[1] - at[1]) < Fingers::RADIUS;
        let mean = |at: [f32; 2]| {
            let picked: Vec<f32> = positions
                .iter()
                .zip(&velocities)
                .filter(|(p, _)| under(p, at))
                .map(|(_, v)| v[0])
                .collect();
            (
                picked.iter().sum::<f32>() / picked.len().max(1) as f32,
                picked.len(),
            )
        };
        let (left, n_left) = mean(touches.each[0].at);
        let (right, n_right) = mean(touches.each[1].at);
        eprintln!(
            "vx under the left finger {left:.3} ({n_left}), the right {right:.3} ({n_right})"
        );
        assert!(n_left > 0 && n_right > 0, "the split found nothing");
        // Each side reads about a third of what one finger alone
        // manages: two fingers pulling apart are trying to open a gap
        // in an incompressible fluid, and the pressure solve is what
        // stops them. The signs and the gap between them are the claim.
        assert!(left < -0.05, "the left finger did not pull left: {left:.3}");
        assert!(
            right > 0.05,
            "the right finger did not pull right: {right:.3}"
        );
        assert!(
            right - left > 0.15,
            "the fingers dragged as one: {left:.3} and {right:.3}"
        );
    }

    // The flat surface gets its ramp a different way from the discs:
    // the splat carries the lens into the field's second channel and
    // the fill divides it back out. This is that path — splat, filter,
    // fill — with the same red-to-blue line the disc test uses.
    #[test]
    fn a_ramp_paints_the_flat_surface_between_its_two_colours() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        let side = 64u32;
        let optics = pack_optics(
            [0.0, -9.81, -0.5],
            sim.extent,
            sim.field_settled,
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        );
        let paint = pack_paint(
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            Lens::Proximity.ends(&f),
            Lens::Proximity.code(),
            0.0,
        );
        let mut seen = std::collections::BTreeSet::new();
        let mut air = 0;
        for px in draw_surface(&device, &queue, &sim, side, paint, optics) {
            if px[..3] == [0, 0, 0] {
                air += 1;
                continue;
            }
            let [b, g, r, _] = px;
            assert_eq!(g, 0, "the ramp left its line: {px:?}");
            assert!(
                (i32::from(r) + i32::from(b) - 255).abs() <= 1,
                "the ramp left its line: {px:?}"
            );
            seen.insert(r);
        }
        eprintln!(
            "flat ramp: {} distinct steps over {} water pixels",
            seen.len(),
            side * side - air
        );
        assert!(air > 0, "the whole screen read as water");
        assert!(seen.len() >= 3, "the ramp painted one colour: {seen:?}");
    }

    // The chase runs on every frame the device draws and no test
    // reached it: the GPU tests all build a Ramp of their own, which
    // takes its first ends whole. Three things it must do — hold off
    // until the field has been read back, hold a zero span open, and
    // open out faster than it closes in.
    #[test]
    fn the_ramp_opens_faster_than_it_closes() {
        let speeds = |lo: f32, hi: f32| {
            let mut stats = [0.0; STATS];
            // Density max is the readback's liveness test.
            stats[3] = sim::REST_DENSITY;
            stats[11] = lo;
            stats[6] = hi;
            stats
        };
        let mut cold = [0.0; STATS];
        cold[6] = 4.0;
        assert_eq!(
            Ramp::new().follow(Lens::Velocity, &cold, SIM_SPACING, 0.0),
            None
        );

        let floor = Lens::Velocity.floor(SIM_SPACING);
        let still = Ramp::new()
            .follow(Lens::Velocity, &speeds(0.0, 0.0), SIM_SPACING, 0.0)
            .expect("live");
        assert_eq!(still, [0.0, floor]);

        let tenth = |from: f32, to: f32| {
            let mut ramp = Ramp::new();
            ramp.follow(Lens::Velocity, &speeds(0.0, from), SIM_SPACING, 0.0);
            ramp.follow(Lens::Velocity, &speeds(0.0, to), SIM_SPACING, 0.1)
                .expect("live")[1]
        };
        // A tenth of a second is two thirds of the opening time
        // constant and a sixth of the closing one.
        let opened = (tenth(4.0, 8.0) - 4.0) / 4.0;
        let closed = (8.0 - tenth(8.0, 4.0)) / 4.0;
        eprintln!("ramp: opened {opened:.3} of the step, closed {closed:.3}");
        assert!((opened - 0.487).abs() < 0.01, "opened {opened:.3}");
        assert!((closed - 0.154).abs() < 0.01, "closed {closed:.3}");
    }

    // Jack, 2026-09-02: "the water is a bit flickery with the gradient
    // on, at near-rest". A ramp between the frame's own two ends puts
    // the solver's own noise on the screen unless the lens's field is
    // quiet and the span has a floor under it. This measures both at
    // once: how far a settled particle walks along its ramp from one
    // frame to the next. The numbers are the M5 record's.
    #[test]
    fn a_settled_pool_holds_its_colours_still() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let g = [0.0, -9.81, -0.5];
        read_stats(&device, &queue, &sim, 7, 600, g, sim::Touches::default());
        const FRAMES: usize = 30;
        let mut walk = [0.0f64; 5];
        let mut last: Option<Vec<[f32; 5]>> = None;
        let mut ends = [[0.0f32; 2]; 5];
        for _ in 0..FRAMES {
            let f = read_stats(&device, &queue, &sim, 7, 1, g, sim::Touches::default());
            for (lens, slot) in LENSES.into_iter().zip(&mut ends) {
                *slot = Ramp::new()
                    .follow(lens, &f, SIM_SPACING, 0.0)
                    .expect("the readback is live after 600 frames");
            }
            let ramp = |lens: usize, v: f32| {
                let [lo, hi] = ends[lens];
                ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
            };
            let velocities = read_vec4(&device, &queue, &sim.velocities, sim.count);
            let smooth_p = read_vec4(&device, &queue, &sim.prev_vel, sim.count);
            let density = read_floats(&device, &queue, &sim.density, sim.count);
            let now: Vec<[f32; 5]> = velocities
                .iter()
                .zip(&smooth_p)
                .zip(&density)
                .map(|((v, p), rho)| {
                    let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                    [
                        ramp(0, speed),
                        ramp(1, v[3]),
                        ramp(2, p[3]),
                        ramp(3, *rho),
                        // The wheel's own place on the screen: the
                        // heading, taken as far as speed has mixed it
                        // in, which is what the eye sees move.
                        v[1].atan2(v[0]) / std::f32::consts::TAU + 0.5,
                    ]
                })
                .collect();
            if let Some(was) = last.replace(now) {
                let now = last.as_ref().expect("just replaced");
                for (a, b) in was.iter().zip(now) {
                    for (lens, step) in walk.iter_mut().enumerate().take(4) {
                        *step += f64::from((b[lens] - a[lens]).abs()) / now.len() as f64;
                    }
                    // The wheel moves in colour, not along a line.
                    let wheel = |q: &[f32; 5]| {
                        let c = hue_rgb(q[4]);
                        std::array::from_fn::<f32, 3, _>(|i| c[i] * q[0] * q[0])
                    };
                    let (x, y) = (wheel(a), wheel(b));
                    let d: f32 = (0..3).map(|i| (x[i] - y[i]).powi(2)).sum();
                    walk[4] += f64::from(d.sqrt()) / now.len() as f64;
                }
            }
        }
        for step in &mut walk {
            *step /= (FRAMES - 1) as f64;
        }
        eprintln!(
            "settled, walked along the ramp a frame: velocity {:.4}, acceleration {:.4}, \
             pressure {:.4}, proximity {:.4}, direction {:.4}",
            walk[0], walk[1], walk[2], walk[3], walk[4]
        );
        for (lens, step) in LENSES.into_iter().zip(walk) {
            assert!(
                step < 0.01,
                "{lens:?} walks {step:.4} of its ramp a frame on settled water"
            );
        }
        // The floor is what holds the speed ramp open: a settled pool
        // spans a fraction of it and paints its low colour, where the
        // frame's own two ends alone would stretch noise over the
        // whole of it.
        let quick = {
            let [lo, hi] = ends[0];
            let f = read_stats(&device, &queue, &sim, 7, 1, g, sim::Touches::default());
            (f[6] - lo) / (hi - lo)
        };
        assert!((0.0..0.1).contains(&quick), "settled v {quick:.3}");
        // Proximity must separate the body from the free surface, and
        // a settled slab has no spray, so its sparsest particle is a
        // surface one, not an isolated drop.
        let f = read_stats(&device, &queue, &sim, 7, 1, g, sim::Touches::default());
        let [lo, hi] = ends[3];
        let fringe = (f[2] - lo) / (hi - lo);
        let body = (f[3] - lo) / (hi - lo);
        assert!(
            body - fringe > 0.25,
            "the free surface reads as body: {fringe:.2} against {body:.2}"
        );
    }

    // Seeds the real lattice, runs one rebuild and density sweep, and
    // reads the stats back: the whole solver chain, not just its
    // compilation.
    #[test]
    fn one_density_sweep_reads_near_rest_density() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(
            &device,
            &queue,
            &sim,
            0,
            1,
            [0.0; 3],
            sim::Touches::default(),
        );
        eprintln!(
            "seeded slab: compr avg {:.4} max {:.4}, rho {:.1}..{:.1}",
            f[0], f[1], f[2], f[3]
        );
        // The half-full slab reads under rest everywhere: free-surface
        // particles lose half their support, and nobody is compressed at
        // seed. A scatter or binding bug reads hundreds of rest
        // densities; dropped neighbours read pure wall fill, under 0.4.
        assert!(f[0] == 0.0 && f[1] == 0.0, "compr {} {}", f[0], f[1]);
        assert!(
            f[2] > 0.4 * sim::REST_DENSITY && f[2] < f[3],
            "min {}",
            f[2]
        );
        assert!(
            f[3] > 0.9 * sim::REST_DENSITY && f[3] < 1.1 * sim::REST_DENSITY,
            "max {}",
            f[3]
        );
        // Nothing has moved or been solved yet.
        assert_eq!((f[4], f[5], f[6]), (0.0, 0.0, 0.0), "pressure, v_max");
        assert_eq!((f[7], f[8]), (293.15, 293.15), "temperature");
        assert_eq!(f[9], 0.0, "clamp count");
    }

    // One bench frame against the CPU: the scan and the density sweep
    // reproduce the reference, so the microbench times real work.
    #[test]
    fn one_bench_frame_matches_the_cpu_reference() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let options = RenderOptions {
            particle_count: 0,
            sprite_radius: 0.0,
            bench_sweeps: 1,
            bench_spacing: 0.0,
            sim_substeps: 0,
            tracers: 0,
            particle_scale: 1.0,
        };
        let mut bench = Bench::new(&device, TEST_EXTENT, &options);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            bench.encode(&mut pass);
        }
        queue.submit(std::iter::once(encoder.finish()));
        let v = bench.validation.take().expect("the first frame validates");
        assert_eq!(
            validate_bench(&device, &queue, &v),
            [true, true],
            "scan, density"
        );
    }

    // One full 120 Hz frame of the solve under gravity: the fluid falls
    // a little, nothing explodes, pressure is non-negative pascals, and
    // the temperature stays within a millikelvin of where it started.
    #[test]
    fn one_frame_of_the_solve_stays_physical() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            1,
            [0.0, -9.81, 0.0],
            sim::Touches::default(),
        );
        eprintln!(
            "one frame: compr avg {:.5} max {:.5}, rho {:.1}..{:.1}, p {:.1}..{:.1}, v {:.4}, T {}..{}, clamps {}",
            f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8], f[9]
        );
        let dt = 1.0 / 120.0 / 7.0;
        let v_clamp = 0.4 * SIM_SPACING / dt;
        assert!(f[6] > 0.0 && f[6] <= v_clamp * 1.001, "v_max {}", f[6]);
        assert!(f[4] >= 0.0 && f[5] < 1.0e6, "pressure {}..{}", f[4], f[5]);
        assert!((f[7] - 293.15).abs() < 1.0e-3, "T min {}", f[7]);
        assert!((f[8] - 293.15).abs() < 1.0e-3, "T max {}", f[8]);
        assert!(f[3] < 2.0 * sim::REST_DENSITY, "rho max {}", f[3]);
    }

    // Field values live in f16's normal range; zero rows decode to
    // exactly zero through the e == 0 arm, and an overflowed store
    // decodes to infinity so is_finite can catch it.
    fn f16(h: u16) -> f32 {
        let e = u32::from(h >> 10) & 0x1f;
        if e == 0 {
            return 0.0;
        }
        if e == 31 {
            return f32::INFINITY;
        }
        f32::from_bits(
            (u32::from(h & 0x8000) << 16) | ((e + 112) << 23) | (u32::from(h & 0x3ff) << 13),
        )
    }

    // The interior plateau of a field: the median of the texels at
    // 60% of the peak or more, with the peak.
    fn plateau(vals: &[f32]) -> (f32, f32) {
        let max = vals.iter().copied().fold(0.0, f32::max);
        let mut interior: Vec<f32> = vals.iter().copied().filter(|v| *v >= 0.6 * max).collect();
        interior.sort_unstable_by(f32::total_cmp);
        (interior[interior.len() / 2], max)
    }

    // One raw draw of the body splat (splat constant 1, the EMA's
    // steady state at rest) into a 32 x 64 target: the target's view,
    // for the filter, and its decoded texels. TEST_EXTENT matches the
    // phone's world within half a percent, and the splat is a
    // point-sampled continuous field, so the small target reads the
    // same plateau the phone renders.
    fn raw_splat(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sim: &Sim,
    ) -> (wgpu::TextureView, Vec<f32>) {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("calibration field"),
            size: wgpu::Extent3d {
                width: 32,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("calibration readback"),
            size: 256 * 64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&sim.body);
            // A solid paint, so the second channel splats zero and the
            // thickness in the first is the whole measurement.
            pass.set_immediates(0, &pack_paint([1.0; 4], [0.0; 4], [0.0, 1.0], 0, 0.0));
            pass.set_blend_constant(wgpu::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            });
            pass.set_bind_group(0, &sim.sprite_bind, &[]);
            pass.draw(0..4, 0..sim.count);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 32,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        let bytes = readback.get_mapped_range(..).expect("mapped");
        let mut vals = Vec::new();
        for row in 0..64 {
            // Two halves a texel now; the thickness is the first.
            let base = row * 256;
            for texel in bytes[base..base + 128].as_chunks::<4>().0 {
                vals.push(f16(u16::from_le_bytes([texel[0], texel[1]])));
            }
        }
        (view, vals)
    }

    // The surface shader divides the field by FIELD_SETTLED to get
    // water thickness. This measures the real settled splat — five
    // upright seconds, then one raw draw — and pins the constant to it.
    #[test]
    fn the_settled_field_matches_the_calibration() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        assert!(f[6] < 0.2, "not settled: v_max {}", f[6]);
        let (view, vals) = raw_splat(&device, &queue, &sim);
        let (plateau, max) = plateau(&vals);
        eprintln!("settled field: plateau {plateau:.3}, max {max:.3}");
        assert!(
            (plateau / FIELD_SETTLED - 1.0).abs() < 0.1,
            "plateau {plateau} vs calibrated {FIELD_SETTLED}"
        );

        // The filter over the same field: blurred thickness and raw
        // texel differences in a half-float store. Recomputed from the
        // stored thickness, the interior differences must agree within
        // f16 rounding of their terms (near 0.016); the border texel's
        // apron reads past the edge, so it is checked for finiteness
        // only.
        let filtered = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("calibration filtered"),
            size: wgpu::Extent3d {
                width: 32,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let filtered_view = filtered.create_view(&Default::default());
        let bind = filter_bind(&device, &sim.filter_layout, &view, &filtered_view);
        let filtered_readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filtered readback"),
            size: 256 * 64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&sim.filter);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_immediates(
                0,
                &pack_optics(
                    [0.0, -9.81, -0.5],
                    sim.extent,
                    sim.field_settled,
                    [0.0; 4],
                    [0.0; 4],
                ),
            );
            let groups = filter_groups([128, 256]);
            pass.dispatch_workgroups(groups[0], groups[1], 1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &filtered,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &filtered_readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 32,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        filtered_readback.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        let filtered_bytes = filtered_readback.get_mapped_range(..).expect("mapped");
        let texels: Vec<[f32; 4]> = filtered_bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|q| {
                let ch = |i: usize| f16(u16::from_le_bytes([q[2 * i], q[2 * i + 1]]));
                [ch(0), ch(1), ch(2), ch(3)]
            })
            .collect();
        let step = [2.0 * TEST_EXTENT[0] / 32.0, 2.0 * TEST_EXTENT[1] / 64.0];
        let aspect = (step[0] / step[1]).powi(2);
        let at = |x: i32, y: i32| texels[y as usize * 32 + x as usize][0];
        let (mut drift, mut edge) = (0.0f32, 0.0f32);
        for y in 0..64i32 {
            for x in 0..32i32 {
                let t = texels[(y * 32 + x) as usize];
                assert!(t.iter().all(|v| v.is_finite()), "texel {x},{y}: {t:?}");
                if x == 0 || y == 0 || x == 31 || y == 63 {
                    continue;
                }
                let d = at(x, y);
                let gx = at(x + 1, y) - at(x - 1, y);
                let gy = at(x, y - 1) - at(x, y + 1);
                let lap = (at(x + 1, y) + at(x - 1, y) - 2.0 * d)
                    + (at(x, y + 1) + at(x, y - 1) - 2.0 * d) * aspect;
                drift = drift
                    .max((t[1] - gx).abs())
                    .max((t[2] - gy).abs())
                    .max((t[3] - lap).abs());
                edge = edge.max(lap.abs());
            }
        }
        let mut blurred: Vec<f32> = texels
            .iter()
            .map(|t| t[0])
            .filter(|v| *v >= 0.6 * max)
            .collect();
        blurred.sort_unstable_by(f32::total_cmp);
        let blurred_plateau = blurred[blurred.len() / 2];
        eprintln!("filtered field: plateau {blurred_plateau:.3}, drift {drift:.4}, edge {edge:.3}");
        assert!(
            (blurred_plateau / plateau - 1.0).abs() < 0.05,
            "blurred plateau {blurred_plateau} vs {plateau}"
        );
        assert!(drift < 0.03, "stored differences drift {drift}");
        assert!(edge > 0.05, "no waterline to test: {edge}");
    }

    // The field is the particle layers per screen area, so a finer
    // lattice settles to a proportionally higher plateau: the scaling
    // every other scale's thickness and edge band rest on (M5 record).
    #[test]
    fn the_settled_field_scales_with_the_spacing() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let spacing = 0.63 * SIM_SPACING;
        let sim = Sim::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            TEST_EXTENT,
            7,
            spacing,
            4096,
            [128, 256],
        );
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, -0.5],
            sim::Touches::default(),
        );
        assert!(f[6] < 0.2, "not settled: v_max {}", f[6]);
        let (_, vals) = raw_splat(&device, &queue, &sim);
        let (plateau, _) = plateau(&vals);
        eprintln!(
            "settled field at {spacing} m: plateau {plateau:.3} vs {:.3}",
            sim.field_settled
        );
        assert!(
            (plateau / sim.field_settled - 1.0).abs() < 0.1,
            "plateau {plateau} vs scaled {}",
            sim.field_settled
        );
    }

    // A second of the solve, phone flat on the desk (gravity into the
    // screen): the state the device diverged in. Settling is allowed;
    // explosion is not.
    #[test]
    fn a_second_of_the_solve_settles_flat() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            120,
            [0.0, 0.0, -9.81],
            sim::Touches::default(),
        );
        eprintln!(
            "one second flat: compr avg {:.5} max {:.5}, rho {:.1}..{:.1}, p {:.1}..{:.1}, v {:.4}, T {}..{}, clamps {}",
            f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8], f[9]
        );
        assert!(f[3] < 1.2 * sim::REST_DENSITY, "rho max {}", f[3]);
        assert!(f[6] < 1.0, "v_max {}", f[6]);
        assert!(f[4] >= 0.0 && f[5] < 1.0e4, "pressure {}..{}", f[4], f[5]);
        assert!(
            (f[7] - 293.15).abs() < 1.0 && (f[8] - 293.15).abs() < 1.0,
            "T {}..{}",
            f[7],
            f[8]
        );
    }
    // Fifteen seconds upright — the deepest column, the hydrostatic
    // ringing case Jack saw flitter in. Still sloshing is fine;
    // explosion or runaway pressure is not.
    #[test]
    fn fifteen_seconds_upright_stay_bounded() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            1800,
            [0.0, -9.81, 0.0],
            sim::Touches::default(),
        );
        eprintln!(
            "fifteen seconds upright: compr avg {:.5} max {:.5}, rho {:.1}..{:.1}, p {:.1}..{:.1}, v {:.4}, T {}..{}, clamps {}",
            f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8], f[9]
        );
        assert!(f[3] < 1.2 * sim::REST_DENSITY, "rho max {}", f[3]);
        assert!(f[6] < 1.0, "v_max {}", f[6]);
        assert!(f[4] >= 0.0 && f[5] < 1.0e4, "pressure {}..{}", f[4], f[5]);
    }
    #[test]
    fn the_charge_rises_with_motion_and_decays_at_rest() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        // A second of hard sideways force churns the box and charges
        // the tracers; five settled seconds must drain most of it
        // (exp(-5/T_CHARGE) plus respawns at resting particles).
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            120,
            [-6.0, -9.81, 0.0],
            sim::Touches::default(),
        );
        let mean = |ts: &[[f32; 4]]| ts.iter().map(|t| t[3]).sum::<f32>() / ts.len() as f32;
        let kicked = mean(&read_tracers(&device, &queue, &sim));
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [0.0, -9.81, 0.0],
            sim::Touches::default(),
        );
        let rested = mean(&read_tracers(&device, &queue, &sim));
        eprintln!("charge kicked {kicked:.3}, rested {rested:.3}");
        assert!(kicked > 0.05, "kicked {kicked}");
        assert!(rested < kicked * 0.6, "rested {rested} vs kicked {kicked}");
    }

    fn read_tracers(device: &wgpu::Device, queue: &wgpu::Queue, sim: &Sim) -> Vec<[f32; 4]> {
        let size = sim.tracer_count as u64 * 8;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tracer staging"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&sim.tracers, 0, &staging, 0, size);
        queue.submit(std::iter::once(encoder.finish()));
        staging.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        let bytes = staging.get_mapped_range(..).expect("mapped");
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| {
                let u = c.as_chunks::<4>().0;
                sim::unpack_tracer(
                    [u32::from_le_bytes(u[0]), u32::from_le_bytes(u[1])],
                    TEST_EXTENT,
                )
            })
            .collect()
    }

    // Gravity at 45 degrees in the screen plane pools the fluid into a
    // corner, the worst case for the additive wall fill: Jack reports
    // the jitter is strongest there (2026-08-31).
    #[test]
    fn fifteen_seconds_in_a_corner_stay_bounded() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(
            &device,
            &queue,
            &sim,
            7,
            1800,
            [-6.94, -6.94, 0.0],
            sim::Touches::default(),
        );
        eprintln!(
            "corner: compr avg {:.5} max {:.5}, rho {:.1}..{:.1}, p {:.1}..{:.1}, v {:.4}, T {}..{}, clamps {}",
            f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8], f[9]
        );
        assert!(f[3] < 1.2 * sim::REST_DENSITY, "rho max {}", f[3]);
        assert!(f[6] < 1.0, "v_max {}", f[6]);
        assert!(f[4] >= 0.0 && f[5] < 1.0e4, "pressure {}..{}", f[4], f[5]);
    }

    // After five seconds of gravity toward the -x wall the fluid pools
    // left of centre. One advect pass with dt above TAU recycles every
    // tracer, so each must land at a solver particle, inside the pool;
    // with recycling broken, riders stranded right of the pool stay
    // put. Pins the collapse Jack recorded on 2026-08-31.
    #[test]
    fn recycling_regathers_tracers_into_the_fluid() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        read_stats(
            &device,
            &queue,
            &sim,
            7,
            600,
            [-9.81, 0.0, 0.0],
            sim::Touches::default(),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_bind_group(0, &sim.tracer_bind, &[]);
            pass.set_pipeline(&sim.advect);
            pass.set_immediates(
                0,
                &sim::pack_step(
                    [-9.81, 0.0, 0.0],
                    [0.0; 3],
                    [0.0; 3],
                    6.0,
                    0.0,
                    601,
                    sim::Touches::default(),
                ),
            );
            pass.dispatch_workgroups(sim.tracer_count.div_ceil(256), 1, 1);
        }
        queue.submit(std::iter::once(encoder.finish()));
        let tracers = read_tracers(&device, &queue, &sim);
        let strays = tracers
            .iter()
            .filter(|t| t[0] > crate::WORLD_SCALE * 0.015)
            .count();
        let max_x = tracers.iter().fold(f32::MIN, |m, t| m.max(t[0]));
        eprintln!("strays {strays}, max x {max_x:.4}");
        assert_eq!(strays, 0, "max x {max_x}");
    }
}
