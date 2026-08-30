//! The M3 simulation substrate: slab geometry, the neighbour grid's cell
//! arithmetic, the cubic-spline kernel, and lattice seeding. The WGSL in
//! sim_*.wgsl mirrors the cell arithmetic exactly; a divergence is a bug.

/// Rest density of water at 20 °C, kg/m³.
pub(crate) const REST_DENSITY: f32 = 998.2;

/// Interior depth of the reference device, metres (iPhone 13 Pro Max,
/// 7.65 mm). A second device would bring its own.
pub(crate) const SLAB_DEPTH: f32 = 0.007_65;

/// The grid: cells of one support radius over the slab, so a particle's
/// neighbours live in its 27 surrounding cells.
pub(crate) struct Grid {
    pub min: [f32; 3],
    pub cell: f32,
    pub dims: [u32; 3],
}

impl Grid {
    /// `extent` is the box half-width and half-height (M2's convention);
    /// the slab spans ±SLAB_DEPTH/2 in z. One guard cell each side keeps
    /// clamped positions in range.
    pub fn new(extent: [f32; 2], support: f32) -> Grid {
        let half = [extent[0], extent[1], SLAB_DEPTH * 0.5];
        let dims = std::array::from_fn(|i| (2.0 * half[i] / support).ceil() as u32 + 2);
        Grid {
            min: std::array::from_fn(|i| -half[i] - support),
            cell: support,
            dims,
        }
    }

    pub fn cell_count(&self) -> u32 {
        self.dims[0] * self.dims[1] * self.dims[2]
    }

    pub fn cell_of(&self, pos: [f32; 3]) -> u32 {
        let coord: [u32; 3] = std::array::from_fn(|i| {
            (((pos[i] - self.min[i]) / self.cell) as u32).min(self.dims[i] - 1)
        });
        (coord[2] * self.dims[1] + coord[1]) * self.dims[0] + coord[0]
    }
}

/// Cubic-spline kernel, 3D normalisation, zero at and beyond the support
/// radius 2h. The CPU reference the tests pin the WGSL against; the
/// shaders carry the runtime copy.
#[cfg(test)]
fn kernel(r: f32, h: f32) -> f32 {
    let q = r / h;
    let sigma = 1.0 / (std::f32::consts::PI * h * h * h);
    if q < 1.0 {
        sigma * (1.0 - 1.5 * q * q * (1.0 - 0.5 * q))
    } else if q < 2.0 {
        let t = 2.0 - q;
        sigma * 0.25 * t * t * t
    } else {
        0.0
    }
}

/// A jittered lattice at spacing `d` filling the slab's lower `fill` of
/// height, full width and depth, centred; four floats a particle (x, y, z,
/// then a zero the shader ignores for vec4 alignment).
pub(crate) fn seed_slab(spacing: f32, extent: [f32; 2], fill: f32) -> Vec<f32> {
    let size = [
        2.0 * extent[0] - spacing,
        (2.0 * extent[1] - spacing) * fill,
        SLAB_DEPTH - spacing,
    ];
    let n: [u32; 3] = std::array::from_fn(|i| ((size[i] / spacing) as u32).max(1));
    let origin = [
        -0.5 * n[0] as f32 * spacing,
        -extent[1] + 0.5 * spacing,
        -0.5 * n[2] as f32 * spacing,
    ];
    let mut out = Vec::with_capacity((n[0] * n[1] * n[2]) as usize * 4);
    let jitter = |h: u32| (h as f32 / u32::MAX as f32 - 0.5) * 0.2;
    for iz in 0..n[2] {
        for iy in 0..n[1] {
            for ix in 0..n[0] {
                let i = (iz * n[1] + iy) * n[0] + ix;
                out.extend_from_slice(&[
                    origin[0] + (ix as f32 + 0.5 + jitter(hash(i * 3))) * spacing,
                    origin[1] + (iy as f32 + jitter(hash(i * 3 + 1))) * spacing,
                    origin[2] + (iz as f32 + 0.5 + jitter(hash(i * 3 + 2))) * spacing,
                    0.0,
                ]);
            }
        }
    }
    out
}

/// The Step immediates block in sim_solve.wgsl: force on 16-byte vec3
/// alignment, dt in the vec3 tail slot, the CFL clamp speed after it,
/// then padding to the struct's 32-byte size.
pub(crate) fn pack_step(force: [f32; 3], dt: f32, v_clamp: f32) -> [u8; 32] {
    let mut raw = [0u8; 32];
    for (slot, v) in [force[0], force[1], force[2], dt, v_clamp]
        .into_iter()
        .enumerate()
    {
        raw[slot * 4..slot * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    raw
}

/// Kernel mass in the half-space beyond a flat wall at distance `d`,
/// as a fraction of the whole kernel: multiply by the rest density to
/// fill the missing region with fluid at rest. Closed-form integral of
/// the cubic spline over a plane-clipped support ball, in t = d/h;
/// the quadrature test pins every coefficient. The runtime copy lives
/// in sim_density.wgsl; this is the CPU reference the tests hold it to.
#[cfg(test)]
pub(crate) fn wall_density(t: f32) -> f32 {
    if t >= 2.0 {
        return 0.0;
    }
    if t < 1.0 {
        let t3 = t * t * t;
        0.5 + t * (-0.7) + t3 * (1.0 / 3.0 + t * t * (-0.15 + t * 0.05))
    } else {
        let t3 = t * t * t;
        8.0 / 15.0 + t * (-0.8) + t3 * (2.0 / 3.0 + t * (-0.5 + t * (0.15 - t / 60.0)))
    }
}

/// Magnitude of the kernel-gradient integral over the same clipped
/// region, times h: divide by h and point along the wall normal. Closed
/// form via the divergence theorem — the kernel's flux through the wall
/// plane.
#[cfg(test)]
pub(crate) fn wall_gradient(t: f32) -> f32 {
    if t >= 2.0 {
        return 0.0;
    }
    let t2 = t * t;
    if t < 1.0 {
        0.7 + t2 * (-1.0 + t2 * (0.75 - t * 0.3))
    } else {
        0.8 + t2 * (-2.0 + t * (2.0 + t * (-0.75 + t * 0.1)))
    }
}

fn hash(x: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^ (h >> 13)
}

/// The SimParams uniform both sim shaders declare; 48 bytes, vec3 slots
/// padded per WGSL uniform alignment.
pub(crate) fn pack_sim_params(grid: &Grid, count: u32, h: f32, mass: f32) -> [u8; 48] {
    let mut raw = [0u8; 48];
    let mut put_f = |off: usize, v: f32| raw[off..off + 4].copy_from_slice(&v.to_le_bytes());
    for (i, v) in grid.min.iter().enumerate() {
        put_f(i * 4, *v);
    }
    put_f(12, grid.cell);
    put_f(32, h);
    put_f(36, mass);
    put_f(40, REST_DENSITY);
    for (i, v) in grid.dims.iter().enumerate() {
        raw[16 + i * 4..20 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    raw[28..32].copy_from_slice(&count.to_le_bytes());
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTENT: [f32; 2] = [0.0356, 0.077];

    #[test]
    fn the_kernel_integrates_to_one() {
        let h = 0.004;
        let step = h / 20.0;
        let mut sum = 0.0f64;
        let n = (2.0 * h / step) as i32 + 1;
        for ix in -n..=n {
            for iy in -n..=n {
                for iz in -n..=n {
                    let r = ((ix * ix + iy * iy + iz * iz) as f32).sqrt() * step;
                    sum += f64::from(kernel(r, h)) * f64::from(step).powi(3);
                }
            }
        }
        assert!((sum - 1.0).abs() < 0.01, "kernel integral {sum}");
    }

    #[test]
    fn a_lattice_at_spacing_d_recovers_rest_density() {
        // Mass = rho0 * d^3 must make an interior particle's SPH density
        // read rest density; this pins mass, kernel and h together.
        let d = 0.002f32;
        let h = 1.2 * d;
        let mass = REST_DENSITY * d * d * d;
        let mut rho = 0.0;
        for ix in -3i32..=3 {
            for iy in -3i32..=3 {
                for iz in -3i32..=3 {
                    let r = ((ix * ix + iy * iy + iz * iz) as f32).sqrt() * d;
                    rho += mass * kernel(r, h);
                }
            }
        }
        let err = (rho - REST_DENSITY).abs() / REST_DENSITY;
        assert!(err < 0.02, "lattice density {rho}, err {err}");
    }

    #[test]
    fn every_seeded_particle_lands_in_a_valid_cell() {
        let grid = Grid::new(EXTENT, 0.0048);
        let floats = seed_slab(0.002, EXTENT, 0.5);
        assert!(!floats.is_empty());
        for p in floats.as_chunks::<4>().0 {
            assert!(p[0].abs() <= EXTENT[0] && p[1].abs() <= EXTENT[1]);
            assert!(p[2].abs() <= SLAB_DEPTH * 0.5);
            assert!(grid.cell_of([p[0], p[1], p[2]]) < grid.cell_count());
        }
    }

    #[test]
    fn neighbouring_positions_map_to_adjacent_cells() {
        let grid = Grid::new(EXTENT, 0.0048);
        let a = grid.cell_of([0.0, 0.0, 0.0]);
        let b = grid.cell_of([grid.cell, 0.0, 0.0]);
        let c = grid.cell_of([0.0, grid.cell, 0.0]);
        assert_eq!(b, a + 1);
        assert_eq!(c, a + grid.dims[0]);
    }

    // Both wall integrals reduce to 1D radial integrals: the cap area
    // 2*pi*r*(r-d) for the mass, the plane flux 2*pi*r for the gradient.
    fn wall_quadrature(d: f32, gradient: bool) -> f32 {
        let n = 20_000;
        let dr = (2.0 - d) / n as f32;
        (0..n)
            .map(|i| {
                let r = d + (i as f32 + 0.5) * dr;
                let cap = if gradient { 1.0 } else { r - d };
                kernel(r, 1.0) * 2.0 * std::f32::consts::PI * r * cap * dr
            })
            .sum()
    }

    #[test]
    fn wall_integrals_match_quadrature() {
        for i in 0..40 {
            let d = i as f32 * 0.05;
            assert!(
                (wall_density(d) - wall_quadrature(d, false)).abs() < 1e-4,
                "V({d})"
            );
            assert!(
                (wall_gradient(d) - wall_quadrature(d, true)).abs() < 1e-4,
                "G({d})"
            );
        }
        assert_eq!(wall_density(2.0), 0.0);
        assert_eq!(wall_gradient(2.0), 0.0);
    }

    // The wall term must close the gap between a half-space lattice and
    // the full lattice: comparing against the bulk sum, not the rest
    // density, isolates the wall integral from the ~2% discrete-sum bias
    // the bulk test already tolerates. The residual is the continuum
    // fill standing in for a discrete lattice.
    #[test]
    fn the_wall_term_closes_the_half_lattice_gap() {
        let d = 0.0025_f32;
        let h = 1.2 * d;
        let mass = REST_DENSITY * d * d * d;
        for layer in 0..4 {
            let z0 = (layer as f32 + 0.5) * d;
            let mut half = 0.0;
            let mut bulk = 0.0;
            for x in -3i32..=3 {
                for y in -3i32..=3 {
                    for z in -6i32..=6 {
                        let (dx, dy) = (x as f32 * d, y as f32 * d);
                        let dz = (z as f32 + 0.5) * d - z0;
                        let w = mass * kernel((dx * dx + dy * dy + dz * dz).sqrt(), h);
                        bulk += w;
                        if z >= 0 {
                            half += w;
                        }
                    }
                }
            }
            // The 3% bound is honest, not loose: the fill overshoots the
            // missing lattice by ~2.2% at the wall-adjacent layer, pure
            // midpoint-rule undershoot of the steep kernel at distance d.
            // A flowing fluid decorrelates and matches the continuum; the
            // bias belongs to the pristine seeded state alone.
            let err = (half + REST_DENSITY * wall_density(z0 / h) - bulk) / bulk;
            assert!(err.abs() < 0.03, "layer {layer}: err {err}");
        }
    }

    #[test]
    fn step_immediates_land_at_the_shader_offsets() {
        let raw = pack_step([1.0, 2.0, 3.0], 4.0, 5.0);
        for (slot, want) in [1.0f32, 2.0, 3.0, 4.0, 5.0].into_iter().enumerate() {
            let got = f32::from_le_bytes(raw[slot * 4..slot * 4 + 4].try_into().unwrap());
            assert_eq!(got, want);
        }
        assert_eq!(&raw[20..32], &[0u8; 12]);
    }

    #[test]
    fn sim_params_land_at_the_shader_offsets() {
        let grid = Grid {
            min: [1.0, 2.0, 3.0],
            cell: 4.0,
            dims: [5, 6, 7],
        };
        let raw = pack_sim_params(&grid, 8, 9.0, 10.0);
        let f = |off: usize| f32::from_le_bytes(raw[off..off + 4].try_into().unwrap());
        let u = |off: usize| u32::from_le_bytes(raw[off..off + 4].try_into().unwrap());
        assert_eq!([f(0), f(4), f(8), f(12)], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!([u(16), u(20), u(24), u(28)], [5, 6, 7, 8]);
        assert_eq!([f(32), f(36), f(40)], [9.0, 10.0, REST_DENSITY]);
        assert_eq!(&raw[44..48], &[0u8; 4]);
    }
}
