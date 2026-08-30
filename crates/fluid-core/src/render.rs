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

/// The reference device's 458 ppi (CLAUDE.md section 5); a second device
/// would bring its own density in through the shell.
const METRES_PER_PIXEL: f32 = 0.0254 / 458.0;

/// One integration step never exceeds two 60 Hz frames, so a resume after
/// a pause cannot fling the particles.
const MAX_DT: f32 = 1.0 / 30.0;

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

/// One g moves a channel a quarter of its range, so a tilt is unmissable
/// and a hard shake saturates; the whole tint sits at quarter brightness so
/// the sprites carry the scene. z drives blue with the sign flipped so rest
/// reads as water, not mud.
fn clear_colour(force: [f32; 3]) -> wgpu::Color {
    let channel = |f: f32, sign: f32| {
        f64::from((0.5 + sign * f / (4.0 * crate::STANDARD_GRAVITY)).clamp(0.0, 1.0)) * 0.25
    };
    wgpu::Color {
        r: channel(force[0], 1.0),
        g: channel(force[1], 1.0),
        b: channel(force[2], -1.0),
        a: 1.0,
    }
}

/// wgpu's native adapter and device futures are ready on the first poll
/// (verified in the wgpu 30.0.1 source); Pending means that contract
/// changed.
fn ready<T>(fut: impl Future<Output = T>) -> T {
    match pin!(fut).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => unreachable!("wgpu future was not ready"),
    }
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
}

enum Mode {
    Demo(Box<Particles>),
    Bench(Box<Bench>),
    Sim(Box<Sim>),
}

/// The M3 record fixes the starting spacing; the ramp revisits it.
const SIM_SPACING: f32 = 0.0025;

struct Sim {
    clear_counts: wgpu::ComputePipeline,
    count_cells: wgpu::ComputePipeline,
    scan_single: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    density_factor: wgpu::ComputePipeline,
    forces_eval: wgpu::ComputePipeline,
    forces_apply: wgpu::ComputePipeline,
    div_kappa: wgpu::ComputePipeline,
    div_apply: wgpu::ComputePipeline,
    den_kappa: wgpu::ComputePipeline,
    den_apply: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    reduce_stats: wgpu::ComputePipeline,
    sprites: wgpu::RenderPipeline,
    grid_bind: wgpu::BindGroup,
    scan_bind: wgpu::BindGroup,
    solve_bind: wgpu::BindGroup,
    sprite_bind: wgpu::BindGroup,
    stats_src: wgpu::Buffer,
    stats_staging: [StagingSlot; 3],
    stats: [f32; 10],
    count: u32,
    cell_groups: u32,
    max_substeps: u32,
    substeps_used: u32,
}

impl Sim {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        extent: [f32; 2],
        substeps: u32,
    ) -> Sim {
        let h = 1.2 * SIM_SPACING;
        let grid = sim::Grid::new(extent, 2.0 * h);
        let cells = grid.cell_count();
        // scan_single serialises 32 cells per thread in one workgroup.
        assert!(cells <= 8_192, "the solver scan covers 8,192 cells");
        let seeded = sim::seed_slab(SIM_SPACING, extent, 0.5);
        let count = (seeded.len() / 4) as u32;
        eprintln!("sim: {count} particles, {cells} cells, {substeps} substeps");

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
            label: Some("sim positions"),
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
        // wgpu zero-initialises buffers: the fluid starts at rest.
        let velocities = storage("sim velocities", u64::from(count) * 16, none);
        let counts = storage("sim counts", u64::from(cells) * 4, none);
        let starts = storage("sim starts", u64::from(cells) * 4, none);
        let cursors = storage("sim cursors", u64::from(cells) * 4, none);
        let sorted = storage("sim sorted", u64::from(count) * 4, none);
        let density = storage("sim density", u64::from(count) * 4, none);
        let alpha = storage("sim alpha", u64::from(count) * 4, none);
        let kappa = storage("sim kappa", u64::from(count) * 4, none);
        let pressure = storage("sim pressure", u64::from(count) * 4, none);
        let prev_pressure = storage("sim prev pressure", u64::from(count) * 4, none);
        let clamps = storage("sim clamps", 4, none);
        let accel = storage("sim accel", u64::from(count) * 16, none);
        let stats_src = storage("sim stats", 40, wgpu::BufferUsages::COPY_SRC);
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
        let stats_staging = std::array::from_fn(|_| StagingSlot {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sim stats staging"),
                size: 40,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            state: Arc::new(AtomicU8::new(SLOT_FREE)),
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim params"),
            size: 48,
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
                sim::REST_DENSITY * SIM_SPACING * SIM_SPACING * SIM_SPACING,
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
                ro(3),
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
                entry(3, &counts),
                entry(4, &starts),
                entry(5, &sorted),
                entry(6, &density),
                entry(7, &alpha),
                entry(8, &kappa),
                entry(9, &pressure),
                entry(10, &prev_pressure),
                entry(11, &temperature),
                entry(12, &stats_src),
                entry(13, &clamps),
                entry(14, &accel),
            ],
        );
        let sprite_bind = bind(
            "sim sprites",
            &sprite_layout,
            &[
                entry(0, &params),
                entry(1, &positions),
                entry(2, &velocities),
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
        let pipe_layout = |label, layout: &wgpu::BindGroupLayout, immediate_size: u32| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size,
            })
        };
        let grid_pl = pipe_layout("sim grid", &grid_layout, 0);
        let scan_pl = pipe_layout("sim scan", &scan_layout, 0);
        let solve_pl = pipe_layout("sim solve", &solve_layout, 32);
        let sprite_pl = pipe_layout("sim sprites", &sprite_layout, 0);
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
            clear_counts: pipeline(&grid_pl, &grid_module, "clear_counts"),
            count_cells: pipeline(&grid_pl, &grid_module, "count"),
            scan_single: pipeline(&scan_pl, &scan_module, "scan_single"),
            scatter: pipeline(&grid_pl, &grid_module, "scatter"),
            density_factor: pipeline(&solve_pl, &solve_module, "density_factor"),
            forces_eval: pipeline(&solve_pl, &solve_module, "forces_eval"),
            forces_apply: pipeline(&solve_pl, &solve_module, "forces_apply"),
            div_kappa: pipeline(&solve_pl, &solve_module, "div_kappa"),
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
            grid_bind,
            scan_bind,
            solve_bind,
            sprite_bind,
            stats_src,
            stats_staging,
            stats: [0.0; 10],
            count,
            cell_groups: cells.div_ceil(256),
            max_substeps: substeps,
            substeps_used: 0,
        }
    }
}

struct Validation {
    seeded: Vec<f32>,
    grid: sim::Grid,
    starts: wgpu::Buffer,
    staging: wgpu::Buffer,
}

struct Bench {
    clear_counts: wgpu::ComputePipeline,
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
        let spacing = options.bench_spacing.clamp(0.001, 0.01);
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
        let density = storage("density", u64::from(count) * 4, none);
        let counts = storage("counts", u64::from(cells) * 4, none);
        let starts = storage("starts", u64::from(cells) * 4, wgpu::BufferUsages::COPY_SRC);
        let cursors = storage("cursors", u64::from(cells) * 4, none);
        let block_sums = storage("block_sums", u64::from(cells.div_ceil(256)) * 4, none);
        let sorted = storage("sorted", u64::from(count) * 4, none);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim params"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        params
            .get_mapped_range_mut(..)
            .expect("mapped at creation")
            .copy_from_slice(&sim::pack_sim_params(&grid, count, h, mass));
        params.unmap();
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scan staging"),
            size: u64::from(cells) * 4,
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
                entry(2, &counts),
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
            clear_counts: pipeline(&grid_pl, &grid_module, "clear_counts"),
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
                starts,
                staging,
            }),
        }
    }

    /// Dispatch order is the data dependency; WebGPU orders storage
    /// writes between dispatches in one pass.
    fn encode(&self, pass: &mut wgpu::ComputePass<'_>) {
        pass.set_bind_group(0, &self.grid_bind, &[]);
        pass.set_pipeline(&self.clear_counts);
        pass.dispatch_workgroups(self.cell_groups, 1, 1);
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

/// Blocks once, on the bench's first frame, never on the steady path.
fn validate_scan(device: &wgpu::Device, queue: &wgpu::Queue, v: &Validation) {
    let cells = v.grid.cell_count() as usize;
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_buffer_to_buffer(&v.starts, 0, &v.staging, 0, (cells * 4) as u64);
    queue.submit(std::iter::once(encoder.finish()));
    let done = Arc::new(AtomicU8::new(0));
    let flag = done.clone();
    v.staging.map_async(wgpu::MapMode::Read, .., move |r| {
        flag.store(if r.is_ok() { 1 } else { 2 }, Ordering::Release);
    });
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    if done.load(Ordering::Acquire) != 1 {
        eprintln!("scan validation: map failed");
        return;
    }
    let gpu: Vec<u32> = {
        let bytes = v.staging.get_mapped_range(..).expect("mapped");
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect()
    };
    v.staging.unmap();
    let mut expect = vec![0u32; cells];
    for p in v.seeded.as_chunks::<4>().0 {
        expect[v.grid.cell_of([p[0], p[1], p[2]]) as usize] += 1;
    }
    let mut run = 0u32;
    for e in &mut expect {
        let c = *e;
        *e = run;
        run += c;
    }
    eprintln!(
        "scan validation: {}",
        if gpu == expect { "PASS" } else { "FAIL" }
    );
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
    pub substeps: u32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    interval_us: Ring,
    encode_us: Ring,
    gpu_us: Ring,
    gpu_timing: Option<GpuTiming>,
    mode: Mode,
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
            // IMMEDIATES is unconditional: Metal always grants it, and
            // the M3 record forbids a fallback branch no run reaches.
            required_features: wgpu::Features::IMMEDIATES
                | if timestamps {
                    wgpu::Features::TIMESTAMP_QUERY
                } else {
                    wgpu::Features::empty()
                },
            // WebGPU's default limits overshoot small adapters (the
            // simulator offers 15 inter-stage variables, the default
            // asks 16), so start from downlevel and raise what the code
            // binds: the sim layouts hold five storage buffers a stage.
            required_limits: wgpu::Limits {
                max_storage_buffers_per_shader_stage: 14,
                max_immediate_size: 32,
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

        let extent = [
            width as f32 * 0.5 * METRES_PER_PIXEL,
            height as f32 * 0.5 * METRES_PER_PIXEL,
        ];
        let mode = if options.bench_sweeps > 0 {
            Mode::Bench(Box::new(Bench::new(&device, extent, &options)))
        } else if options.sim_substeps > 0 {
            Mode::Sim(Box::new(Sim::new(
                &device,
                config.format,
                extent,
                options.sim_substeps,
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
            encode_us: Ring::new(),
            gpu_us: Ring::new(),
            gpu_timing,
            mode,
            frames: 0,
            last_frame_ms: 0.0,
        })
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
        self.surface.configure(&self.device, &self.config);
    }

    /// `now_ms` is `CADisplayLink.timestamp` in milliseconds; only
    /// differences are taken.
    pub fn frame(&mut self, sample: MotionSample, now_ms: f64) {
        // A gap after a pause is not a frame interval; half a second cuts
        // off resumes without hiding real hitches.
        let interval_us = ((now_ms - self.last_frame_ms) * 1_000.0) as f32;
        if self.frames > 0 && interval_us < 500_000.0 {
            self.interval_us.push(interval_us);
        }
        self.last_frame_ms = now_ms;
        let dt = if self.frames == 0 {
            0.0
        } else {
            (interval_us / 1_000_000.0).clamp(0.0, MAX_DT)
        };
        let force = sample.body_force();
        if let Mode::Sim(s) = &mut self.mode {
            // CFL: dt <= 0.4 d / v_max, from the one-frame-stale v_max
            // the stats readback drained. The GPU clamp enforces the dt
            // actually encoded.
            s.substeps_used = if dt > 0.0 {
                ((dt * s.stats[6] / (0.4 * SIM_SPACING)).ceil() as u32).clamp(1, s.max_substeps)
            } else {
                1
            };
        }

        let started = std::time::Instant::now();

        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };

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
                Mode::Sim(s) => {
                    let n = s.substeps_used;
                    let dt_sub = dt / n as f32;
                    let v_clamp = if dt_sub > 0.0 {
                        0.4 * SIM_SPACING / dt_sub
                    } else {
                        f32::MAX
                    };
                    let step = sim::pack_step(force, dt_sub, v_clamp);
                    let particles = s.count.div_ceil(256);
                    for _ in 0..n {
                        pass.set_bind_group(0, &s.grid_bind, &[]);
                        pass.set_pipeline(&s.clear_counts);
                        pass.dispatch_workgroups(s.cell_groups, 1, 1);
                        pass.set_pipeline(&s.count_cells);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_bind_group(0, &s.scan_bind, &[]);
                        pass.set_pipeline(&s.scan_single);
                        pass.dispatch_workgroups(1, 1, 1);
                        pass.set_bind_group(0, &s.grid_bind, &[]);
                        pass.set_pipeline(&s.scatter);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_bind_group(0, &s.solve_bind, &[]);
                        pass.set_pipeline(&s.density_factor);
                        pass.set_immediates(0, &step);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_pipeline(&s.div_kappa);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_pipeline(&s.div_apply);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_pipeline(&s.forces_eval);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_pipeline(&s.forces_apply);
                        pass.dispatch_workgroups(particles, 1, 1);
                        for _ in 0..2 {
                            pass.set_pipeline(&s.den_kappa);
                            pass.dispatch_workgroups(particles, 1, 1);
                            pass.set_pipeline(&s.den_apply);
                            pass.dispatch_workgroups(particles, 1, 1);
                        }
                        pass.set_pipeline(&s.integrate);
                        pass.dispatch_workgroups(particles, 1, 1);
                    }
                    pass.set_pipeline(&s.reduce_stats);
                    pass.dispatch_workgroups(1, 1, 1);
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_colour(force)),
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
                Mode::Sim(s) => {
                    pass.set_pipeline(&s.sprites);
                    pass.set_bind_group(0, &s.sprite_bind, &[]);
                    pass.draw(0..4, 0..s.count);
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
                encoder.copy_buffer_to_buffer(&s.stats_src, 0, &s.stats_staging[i].buffer, 0, 40);
            }
            free
        } else {
            None
        };
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
            validate_scan(&self.device, &self.queue, &v);
        }
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
            _ => ([0.0; 10], 0),
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
            substeps,
            frames: self.frames,
            interval_p50_us: self.interval_us.percentile(0.5),
            interval_p99_us: self.interval_us.percentile(0.99),
            interval_max_us: self.interval_us.max(),
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
    fn at_rest_face_up_the_clear_is_calm_blue() {
        let colour = clear_colour(
            MotionSample {
                gravity: [0.0, 0.0, -1.0],
                user_acceleration: [0.0; 3],
            }
            .body_force(),
        );
        assert_eq!((colour.r, colour.g, colour.b), (0.125, 0.125, 0.1875));
    }

    #[test]
    fn a_hard_shake_saturates_instead_of_wrapping() {
        let colour = clear_colour([100.0, -100.0, 0.0]);
        assert_eq!((colour.r, colour.g, colour.b), (0.25, 0.0, 0.125));
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

    fn headless_sim() -> Option<(wgpu::Device, wgpu::Queue, Sim)> {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = ready(instance.request_adapter(&Default::default())) else {
            eprintln!("no GPU adapter; skipping");
            return None;
        };
        let (device, queue) = ready(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::IMMEDIATES,
            required_limits: wgpu::Limits {
                max_storage_buffers_per_shader_stage: 14,
                max_immediate_size: 32,
                ..wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
            },
            ..Default::default()
        }))
        .expect("device");
        let sim = Sim::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            [0.0357, 0.0774],
            7,
        );
        Some((device, queue, sim))
    }

    fn read_stats(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sim: &Sim,
        substeps: u32,
    ) -> [f32; 10] {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stats staging"),
            size: 40,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Gravity along -y, one 120 Hz frame split like the phone would.
        let dt = 1.0 / 120.0 / substeps.max(1) as f32;
        let v_clamp = if substeps == 0 {
            f32::MAX
        } else {
            0.4 * SIM_SPACING / dt
        };
        let step = sim::pack_step(
            [0.0, -9.81, 0.0],
            if substeps == 0 { 0.0 } else { dt },
            v_clamp,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            let particles = sim.count.div_ceil(256);
            for _ in 0..substeps.max(1) {
                pass.set_bind_group(0, &sim.grid_bind, &[]);
                pass.set_pipeline(&sim.clear_counts);
                pass.dispatch_workgroups(sim.cell_groups, 1, 1);
                pass.set_pipeline(&sim.count_cells);
                pass.dispatch_workgroups(particles, 1, 1);
                pass.set_bind_group(0, &sim.scan_bind, &[]);
                pass.set_pipeline(&sim.scan_single);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_bind_group(0, &sim.grid_bind, &[]);
                pass.set_pipeline(&sim.scatter);
                pass.dispatch_workgroups(particles, 1, 1);
                pass.set_bind_group(0, &sim.solve_bind, &[]);
                pass.set_pipeline(&sim.density_factor);
                pass.set_immediates(0, &step);
                pass.dispatch_workgroups(particles, 1, 1);
                if substeps > 0 {
                    pass.set_pipeline(&sim.div_kappa);
                    pass.dispatch_workgroups(particles, 1, 1);
                    pass.set_pipeline(&sim.div_apply);
                    pass.dispatch_workgroups(particles, 1, 1);
                    pass.set_pipeline(&sim.forces_eval);
                    pass.dispatch_workgroups(particles, 1, 1);
                    pass.set_pipeline(&sim.forces_apply);
                    pass.dispatch_workgroups(particles, 1, 1);
                    for _ in 0..2 {
                        pass.set_pipeline(&sim.den_kappa);
                        pass.dispatch_workgroups(particles, 1, 1);
                        pass.set_pipeline(&sim.den_apply);
                        pass.dispatch_workgroups(particles, 1, 1);
                    }
                    pass.set_pipeline(&sim.integrate);
                    pass.dispatch_workgroups(particles, 1, 1);
                }
            }
            pass.set_pipeline(&sim.reduce_stats);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&sim.stats_src, 0, &staging, 0, 40);
        queue.submit(std::iter::once(encoder.finish()));
        staging.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        let bytes = staging.get_mapped_range(..).expect("mapped");
        let mut out = [0.0f32; 10];
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

    // Seeds the real lattice, runs one rebuild and density sweep, and
    // reads the stats back: the whole solver chain, not just its
    // compilation.
    #[test]
    fn one_density_sweep_reads_near_rest_density() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(&device, &queue, &sim, 0);
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

    // One full 120 Hz frame of the solve under gravity: the fluid falls
    // a little, nothing explodes, pressure is non-negative pascals, and
    // the temperature stays within a millikelvin of where it started.
    #[test]
    fn one_frame_of_the_solve_stays_physical() {
        let Some((device, queue, sim)) = headless_sim() else {
            return;
        };
        let f = read_stats(&device, &queue, &sim, 7);
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
}
