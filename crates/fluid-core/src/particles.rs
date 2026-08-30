//! Pure helpers for the particle pass: deterministic seeding, and the
//! uniform block whose layout both shaders declare as `Params`.

/// A jittered lattice over the upper half of the box, poised to collapse at
/// launch; velocities start at zero. Four floats a particle: x, y, vx, vy,
/// in metres and metres per second.
pub(crate) fn seed(count: u32, extent: [f32; 2]) -> Vec<f32> {
    let width = extent[0] * 1.8;
    let height = extent[1] * 0.9;
    let cols = ((count as f32 * width / height).sqrt().ceil()).max(1.0) as u32;
    let rows = count.div_ceil(cols);
    let cell = [width / cols as f32, height / rows as f32];
    let mut out = Vec::with_capacity(count as usize * 4);
    for i in 0..count {
        let jitter = |h: u32| (h as f32 / u32::MAX as f32 - 0.5) * 0.8;
        let x = ((i % cols) as f32 + 0.5 + jitter(hash(i * 2))) * cell[0] - width * 0.5;
        let y = ((i / cols) as f32 + 0.5 + jitter(hash(i * 2 + 1))) * cell[1];
        out.extend_from_slice(&[x, y, 0.0, 0.0]);
    }
    out
}

fn hash(x: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^ (h >> 13)
}

/// Offsets follow WGSL uniform alignment: a vec2 sits on 8 bytes, so the
/// scalars pack into the gaps and the block closes at 32.
pub(crate) fn pack_params(
    force: [f32; 2],
    dt: f32,
    radius: f32,
    extent: [f32; 2],
    count: u32,
) -> [u8; 32] {
    let mut raw = [0u8; 32];
    let floats = [force[0], force[1], dt, radius, extent[0], extent[1]];
    for (slot, v) in floats.into_iter().enumerate() {
        raw[slot * 4..slot * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    raw[24..28].copy_from_slice(&count.to_le_bytes());
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTENT: [f32; 2] = [0.0356, 0.077];

    #[test]
    fn seeding_fills_the_upper_half_inside_the_walls() {
        let floats = seed(10_000, EXTENT);
        assert_eq!(floats.len(), 40_000);
        for p in floats.as_chunks::<4>().0 {
            assert!(p[0].abs() <= EXTENT[0] * 0.9, "x escaped: {}", p[0]);
            assert!(
                p[1] >= 0.0 && p[1] <= EXTENT[1] * 0.9,
                "y escaped: {}",
                p[1]
            );
            assert_eq!([p[2], p[3]], [0.0, 0.0]);
        }
    }

    #[test]
    fn seeding_is_deterministic_and_jittered() {
        assert_eq!(seed(1_000, EXTENT), seed(1_000, EXTENT));
        let floats = seed(1_000, EXTENT);
        let xs: Vec<f32> = floats.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
        assert!(xs.windows(2).any(|w| w[0] != w[1]));
    }

    #[test]
    fn params_land_at_the_shader_offsets() {
        let raw = pack_params([1.0, 2.0], 3.0, 4.0, [5.0, 6.0], 7);
        let float_at = |off: usize| f32::from_le_bytes(raw[off..off + 4].try_into().unwrap());
        assert_eq!(
            [
                float_at(0),
                float_at(4),
                float_at(8),
                float_at(12),
                float_at(16),
                float_at(20)
            ],
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
        );
        assert_eq!(u32::from_le_bytes(raw[24..28].try_into().unwrap()), 7);
    }
}
