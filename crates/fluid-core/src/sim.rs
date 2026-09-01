//! The M3 simulation substrate: slab geometry, the neighbour grid's cell
//! arithmetic, the cubic-spline kernel, and lattice seeding. The WGSL in
//! sim_*.wgsl mirrors the cell arithmetic exactly; a divergence is a bug.

/// Rest density of water at 20 °C, kg/m³.
pub(crate) const REST_DENSITY: f32 = 998.2;

/// Interior depth of the reference device, metres (iPhone 13 Pro Max,
/// 7.65 mm), times the world scale. A second device would bring its own.
pub(crate) const SLAB_DEPTH: f32 = crate::WORLD_SCALE * 0.007_65;

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
/// radius 2h. The CPU reference the tests and the bench's first frame
/// pin the WGSL against; the shaders carry the runtime copy.
pub(crate) fn kernel(r: f32, h: f32) -> f32 {
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

/// Uniform hash-driven seeding over the same region seed_slab fills, for
/// the visual tracers: massless, so no lattice is needed. Two u32 a
/// tracer, at rest, in the packed record sim_tracers.wgsl reads.
pub(crate) fn seed_tracers(count: u32, extent: [f32; 2], fill: f32) -> Vec<u32> {
    let half = [
        extent[0] - 0.001,
        extent[1] - 0.001,
        0.5 * SLAB_DEPTH - 0.001,
    ];
    let unit = |h: u32| h as f32 / u32::MAX as f32;
    let mut out = Vec::with_capacity(count as usize * 2);
    for i in 0..count {
        let pos = [
            (unit(hash(i * 3)) * 2.0 - 1.0) * half[0],
            -half[1] + unit(hash(i * 3 + 1)) * 2.0 * half[1] * fill,
            (unit(hash(i * 3 + 2)) * 2.0 - 1.0) * half[2],
        ];
        out.extend_from_slice(&pack_tracer(pos, 0.0, extent));
    }
    out
}

/// The 8-byte tracer record store_tracer writes in sim_tracers.wgsl:
/// pos.xy quantised over the box half-extent by pack2x16unorm, then
/// pos.z and the speed as the two halves of pack2x16float.
fn pack_tracer(pos: [f32; 3], speed: f32, extent: [f32; 2]) -> [u32; 2] {
    let unorm = |v: f32, e: f32| ((v / e * 0.5 + 0.5).clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
    [
        unorm(pos[0], extent[0]) | (unorm(pos[1], extent[1]) << 16),
        u32::from(half_bits(pos[2])) | (u32::from(half_bits(speed)) << 16),
    ]
}

/// IEEE half bits of a finite value below half's overflow at 65504. The
/// box extent and a tracer's speed stay far under it, so the infinity
/// and NaN encodings have no producer. Rounds to nearest even, as
/// pack2x16float does.
fn half_bits(x: f32) -> u16 {
    let sign = (x.to_bits() >> 16) as u16 & 0x8000;
    let a = x.abs();
    if a < f32::from_bits(113 << 23) {
        // The f32 step at 0.5 is half's subnormal step, so adding 0.5
        // rounds a onto the subnormal grid and the bits then count it.
        let magic = f32::from_bits(126 << 23);
        sign | ((a + magic).to_bits() - magic.to_bits()) as u16
    } else {
        let bits = a.to_bits();
        sign | (((bits + 0x0fff + ((bits >> 13) & 1)) - (112 << 23)) >> 13) as u16
    }
}

/// The inverse of `pack_tracer`: the three position components, then the
/// speed.
#[cfg(test)]
pub(crate) fn unpack_tracer(raw: [u32; 2], extent: [f32; 2]) -> [f32; 4] {
    let unorm = |q: u32, e: f32| (q as f32 / 65535.0 * 2.0 - 1.0) * e;
    [
        unorm(raw[0] & 0xffff, extent[0]),
        unorm(raw[0] >> 16, extent[1]),
        half_value(raw[1] as u16),
        half_value((raw[1] >> 16) as u16),
    ]
}

/// The inverse of `half_bits`, over the same finite domain.
#[cfg(test)]
fn half_value(bits: u16) -> f32 {
    let raw = u32::from(bits);
    let mut o = f32::from_bits(((raw & 0x7fff) << 13) + (112 << 23));
    if raw & 0x7c00 == 0 {
        o = f32::from_bits(o.to_bits() + (1 << 23)) - f32::from_bits(113 << 23);
    }
    f32::from_bits(o.to_bits() | ((raw & 0x8000) << 16))
}

/// The Step immediates block, one layout shared by sim_solve.wgsl and
/// sim_tracers.wgsl: two vec3s on 16-byte alignment — the body force
/// and the box's angular velocity — with dt and the CFL clamp speed in
/// their tail slots, then the angular acceleration with the tracer
/// seed in its tail. Real rotation reaches this from exactly two
/// places, the device frame path and the film harness; every other
/// caller passes zeros.
pub(crate) fn pack_step(
    force: [f32; 3],
    omega: [f32; 3],
    domega: [f32; 3],
    dt: f32,
    v_clamp: f32,
    seed: u32,
) -> [u8; 48] {
    let mut raw = [0u8; 48];
    for (slot, v) in [
        force[0], force[1], force[2], dt, omega[0], omega[1], omega[2], v_clamp, domega[0],
        domega[1], domega[2],
    ]
    .into_iter()
    .enumerate()
    {
        raw[slot * 4..slot * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    raw[44..48].copy_from_slice(&seed.to_le_bytes());
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

/// The SimParams uniform the sim shaders declare, vec3 slots padded per
/// WGSL uniform alignment. The 48-byte head is common; the tail past
/// rho0 holds the solver's hoisted kernel constants (1/h, the cubic
/// spline's 1/(pi h^3), that over h, and the squared support radius),
/// which only sim_solve.wgsl declares.
pub(crate) fn pack_sim_params(grid: &Grid, count: u32, h: f32, mass: f32) -> [u8; 64] {
    let mut raw = [0u8; 64];
    let mut put_f = |off: usize, v: f32| raw[off..off + 4].copy_from_slice(&v.to_le_bytes());
    for (i, v) in grid.min.iter().enumerate() {
        put_f(i * 4, *v);
    }
    put_f(12, grid.cell);
    put_f(32, h);
    put_f(36, mass);
    put_f(40, REST_DENSITY);
    let sigma = 1.0 / (std::f32::consts::PI * h * h * h);
    put_f(44, 1.0 / h);
    put_f(48, sigma);
    put_f(52, sigma / h);
    put_f(56, 4.0 * h * h);
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
    fn tracers_seed_inside_the_fluid_region() {
        let extent = [0.0357, 0.0774];
        let seeded = seed_tracers(1000, extent, 0.5);
        for raw in seeded.as_chunks::<2>().0 {
            let p = unpack_tracer(*raw, extent);
            assert!(p[0].abs() < extent[0] && p[2].abs() < 0.5 * SLAB_DEPTH);
            assert!(p[1] > -extent[1] && p[1] < 0.0 + 0.001);
        }
    }

    #[test]
    fn step_immediates_land_at_the_shader_offsets() {
        let raw = pack_step(
            [1.0, 2.0, 3.0],
            [7.0, 8.0, 9.0],
            [10.0, 11.0, 12.0],
            4.0,
            5.0,
            6,
        );
        let f = |i: usize| f32::from_le_bytes(raw[i..i + 4].try_into().unwrap());
        assert_eq!([f(0), f(4), f(8)], [1.0, 2.0, 3.0], "force");
        assert_eq!(f(12), 4.0, "dt");
        assert_eq!([f(16), f(20), f(24)], [7.0, 8.0, 9.0], "omega");
        assert_eq!(f(28), 5.0, "v_clamp");
        assert_eq!([f(32), f(36), f(40)], [10.0, 11.0, 12.0], "domega");
        assert_eq!(u32::from_le_bytes(raw[44..48].try_into().unwrap()), 6);
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
        let sigma = 1.0 / (std::f32::consts::PI * 729.0);
        assert_eq!(
            [f(44), f(48), f(52), f(56)],
            [1.0 / 9.0, sigma, sigma / 9.0, 324.0]
        );
        assert_eq!(&raw[60..64], &[0u8; 4]);
    }

    // Copies the shader's wedge polynomials; a drifted edit to either
    // side fails here.
    fn wedge_fit(t1: f64, t2: f64) -> f64 {
        if t1 * t1 + t2 * t2 >= 4.0 {
            return 0.0;
        }
        (2.5005807e-01
            + t2 * (-3.4681153e-01
                + t2 * (-4.0986882e-02
                    + t2 * (3.0301620e-01
                        + t2 * (-1.9202736e-01 + t2 * (4.6947582e-02 + t2 * (-3.8543515e-03)))))))
            + t1 * ((-3.4681153e-01
                + t2 * (4.8587248e-01
                    + t2 * (1.0092180e-02
                        + t2 * (-2.9978363e-01 + t2 * (1.5320335e-01 + t2 * (-2.2544282e-02))))))
                + t1 * ((-4.0986882e-02
                    + t2 * (1.0092180e-02
                        + t2 * (-5.4145610e-02 + t2 * (7.9597369e-02 + t2 * (-2.3350373e-02)))))
                    + t1 * ((3.0301620e-01
                        + t2 * (-2.9978363e-01 + t2 * (7.9597369e-02 + t2 * (-8.2431946e-03))))
                        + t1 * ((-1.9202736e-01 + t2 * (1.5320335e-01 + t2 * (-2.3350373e-02)))
                            + t1 * ((4.6947582e-02 + t2 * (-2.2544282e-02))
                                + t1 * (-3.8543515e-03))))))
    }

    fn wedge_d_fit(t1: f64, t2: f64) -> f64 {
        if t1 * t1 + t2 * t2 >= 4.0 {
            return 0.0;
        }
        (-3.4611994e-01
            + t2 * (4.4757673e-01
                + t2 * (1.1932939e-01
                    + t2 * (-4.6460101e-01
                        + t2 * (2.7637253e-01 + t2 * (-6.2846025e-02 + t2 * (4.3871652e-03)))))))
            + t1 * ((-6.3622823e-02
                + t2 * (2.0521006e-01
                    + t2 * (-3.7311339e-01
                        + t2 * (3.7508155e-01 + t2 * (-1.6629713e-01 + t2 * (2.5030129e-02))))))
                + t1 * ((7.8352497e-01
                    + t2 * (-1.2658344e+00
                        + t2 * (5.8898945e-01 + t2 * (-1.0772903e-01 + t2 * (1.3535777e-02)))))
                    + t1 * ((-4.9470084e-01
                        + t2 * (9.1302071e-01 + t2 * (-3.4330355e-01 + t2 * (2.4034609e-02))))
                        + t1 * ((-2.5605481e-02 + t2 * (-2.0067981e-01 + t2 * (5.9854468e-02)))
                            + t1 * ((8.8898623e-02 + t2 * (3.9914838e-03))
                                + t1 * (-1.7788321e-02))))))
    }

    #[test]
    fn wedge_polynomials_match_quadrature() {
        // The cubic kernel in h units, its line integral L along the
        // wedge's free axis, then I2 as the quarter-plane integral of
        // L and D2 as -integral of L along the t1 = const line.
        let w = |q: f64| -> f64 {
            let s = 1.0 / std::f64::consts::PI;
            if q < 1.0 {
                s * (1.0 - 1.5 * q * q * (1.0 - q / 2.0))
            } else if q < 2.0 {
                s * 0.25 * (2.0 - q).powi(3)
            } else {
                0.0
            }
        };
        let n = 1000usize;
        let dq = 2.0 / n as f64;
        let line = |rho: f64| -> f64 {
            let mut sum = 0.0;
            for i in 0..n {
                let z0 = i as f64 * dq;
                sum += (w((rho * rho + z0 * z0).sqrt())
                    + w((rho * rho + (z0 + dq).powi(2)).sqrt()))
                    / 2.0
                    * dq;
            }
            2.0 * sum
        };
        let m = 400usize;
        let du = 2.0 / m as f64;
        for i in 0..=6 {
            for j in 0..=6 {
                let (a, b) = (i as f64 * 0.3, j as f64 * 0.3);
                if a * a + b * b >= 4.0 {
                    continue;
                }
                let mut i2 = 0.0;
                let mut d2 = 0.0;
                for p in 0..m {
                    let u1 = a + (p as f64 + 0.5) * du;
                    if u1 >= 2.0 {
                        continue;
                    }
                    for q in 0..m {
                        let u2 = b + (q as f64 + 0.5) * du;
                        if u2 < 2.0 {
                            i2 += line((u1 * u1 + u2 * u2).sqrt()) * du * du;
                        }
                    }
                }
                for q in 0..m {
                    let u2 = b + (q as f64 + 0.5) * du;
                    if u2 < 2.0 {
                        d2 -= line((a * a + u2 * u2).sqrt()) * du;
                    }
                }
                assert!(
                    (wedge_fit(a, b) - i2).abs() < 1.5e-3,
                    "I2({a},{b}): fit {} quad {i2}",
                    wedge_fit(a, b)
                );
                assert!(
                    (wedge_d_fit(a, b) - d2).abs() < 8e-3,
                    "D2({a},{b}): fit {} quad {d2}",
                    wedge_d_fit(a, b)
                );
            }
        }
    }

    #[test]
    fn the_wedge_term_closes_the_edge_lattice_gap() {
        let d = 0.0025_f32;
        let h = 1.2 * d;
        let mass = REST_DENSITY * d * d * d;
        for (lx, ly) in [(0, 0), (0, 1), (1, 1), (0, 2), (2, 2)] {
            let (x0, y0) = ((lx as f32 + 0.5) * d, (ly as f32 + 0.5) * d);
            let mut quarter = 0.0;
            let mut bulk = 0.0;
            for x in -6i32..=6 {
                for y in -6i32..=6 {
                    for z in -6i32..=6 {
                        let dx = (x as f32 + 0.5) * d - x0;
                        let dy = (y as f32 + 0.5) * d - y0;
                        let dz = z as f32 * d;
                        let w = mass * kernel((dx * dx + dy * dy + dz * dz).sqrt(), h);
                        bulk += w;
                        if x >= 0 && y >= 0 {
                            quarter += w;
                        }
                    }
                }
            }
            // The same midpoint-rule honesty as the half-lattice test,
            // compounded by two walls: the corner-most layer reads
            // +8.6% without the wedge term and +3.4% with it, and the
            // residual is the pristine-lattice bias, not geometry.
            let fill = wall_density(x0 / h) + wall_density(y0 / h)
                - wedge_fit((x0 / h) as f64, (y0 / h) as f64) as f32;
            let err = (quarter + REST_DENSITY * fill - bulk) / bulk;
            assert!(err.abs() < 0.06, "layers ({lx},{ly}): err {err}");
        }
    }

    // Copies the shader's wall_adhesion polynomial; a drifted edit to
    // either side fails here.
    fn wall_adhesion(t: f64) -> f64 {
        let u = (2.0 - t).clamp(0.0, 2.0);
        u * u
            * (1.784_701_5e-3
                + u * (3.339_219_6e-3
                    + u * (-4.339_467_7e-3 + u * (1.671_186_8e-3 + u * (-2.178_875_0e-4)))))
    }

    #[test]
    fn wall_adhesion_polynomial_matches_the_kernel_quadrature() {
        let h = 1.2 * 0.0025_f64;
        let c = 2.0 * h;
        let aker = |r: f64| -> f64 {
            if 2.0 * r < c || r > c {
                return 0.0;
            }
            0.007 / c.powf(3.25) * (-4.0 * r * r / c + 6.0 * r - 2.0 * c).max(0.0).powf(0.25)
        };
        // J(d) = 2 pi * int_d^c u * (int_u^c A(s) ds) du, trapezoid at
        // 4000 points; the same integral the fit in sigma_eff.py used.
        let n = 4000usize;
        let dr = c / n as f64;
        let a: Vec<f64> = (0..=n).map(|i| aker(i as f64 * dr)).collect();
        let mut int_a = vec![0.0f64; n + 1];
        for i in (0..n).rev() {
            int_a[i] = int_a[i + 1] + (a[i] + a[i + 1]) / 2.0 * dr;
        }
        let mut int_j = vec![0.0f64; n + 1];
        for i in (0..n).rev() {
            let (u0, u1) = (i as f64 * dr, (i + 1) as f64 * dr);
            int_j[i] = int_j[i + 1] + (u0 * int_a[i] + u1 * int_a[i + 1]) / 2.0 * dr;
        }
        let j0 = 2.0 * std::f64::consts::PI * int_j[0];
        for step in 0..=40 {
            let d = c * step as f64 / 40.0;
            let idx = (d / dr).round() as usize;
            let quad = 2.0 * std::f64::consts::PI * int_j[idx];
            assert!(
                (wall_adhesion(d / h) - quad).abs() < 0.005 * j0,
                "d/h {}: poly {} quad {}",
                d / h,
                wall_adhesion(d / h),
                quad
            );
        }
    }
}
