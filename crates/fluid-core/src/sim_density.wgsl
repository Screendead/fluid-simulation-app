// SPH density over the 27 neighbour cells, cubic-spline kernel. Mirrors
// sim.rs::kernel exactly; a divergence is a bug.

struct SimParams {
    box_min: vec3f,
    cell: f32,
    dims: vec3u,
    count: u32,
    h: f32,
    mass: f32,
    rho0: f32,
}

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read> positions: array<vec4f>;
@group(0) @binding(2) var<storage, read> counts: array<u32>;
@group(0) @binding(3) var<storage, read> starts: array<u32>;
@group(0) @binding(4) var<storage, read> sorted: array<u32>;
@group(0) @binding(5) var<storage, read_write> density: array<f32>;

fn kernel(r: f32, h: f32) -> f32 {
    let q = r / h;
    let sigma = 1.0 / (3.14159265 * h * h * h);
    if q < 1.0 {
        return sigma * (1.0 - 1.5 * q * q * (1.0 - 0.5 * q));
    }
    if q < 2.0 {
        let t = 2.0 - q;
        return sigma * 0.25 * t * t * t;
    }
    return 0.0;
}

@compute @workgroup_size(256)
fn density_sweep(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let pos = positions[id.x].xyz;
    let base = min(
        vec3u((pos - params.box_min) / params.cell),
        params.dims - vec3u(1u),
    );
    var rho = 0.0;
    for (var dz = -1i; dz <= 1i; dz++) {
        for (var dy = -1i; dy <= 1i; dy++) {
            for (var dx = -1i; dx <= 1i; dx++) {
                let coord = vec3i(base) + vec3i(dx, dy, dz);
                if any(coord < vec3i(0)) || any(coord >= vec3i(params.dims)) {
                    continue;
                }
                let c = (u32(coord.z) * params.dims.y + u32(coord.y)) * params.dims.x
                    + u32(coord.x);
                let end = starts[c] + counts[c];
                for (var k = starts[c]; k < end; k++) {
                    let r = distance(pos, positions[sorted[k]].xyz);
                    rho += params.mass * kernel(r, params.h);
                }
            }
        }
    }
    density[id.x] = rho;
}

@group(0) @binding(6) var<storage, read_write> stats: array<f32, 2>;

// Mirrors sim.rs::wall_density exactly; a divergence is a bug.
fn wall_density(t: f32) -> f32 {
    if t >= 2.0 {
        return 0.0;
    }
    let t3 = t * t * t;
    if t < 1.0 {
        return 0.5 - 0.7 * t + t3 * (1.0 / 3.0 + t * t * (-0.15 + 0.05 * t));
    }
    return 8.0 / 15.0 - 0.8 * t + t3 * (2.0 / 3.0 + t * (-0.5 + t * (0.15 - t / 60.0)));
}

// Density with the six analytic wall terms: the truncated support at a
// flat wall is filled with rest-density fluid in closed form. Additive
// per wall; the M3 record carries the measured corner residual.
@compute @workgroup_size(256)
fn density_walls(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let pos = positions[id.x].xyz;
    let base = min(
        vec3u((pos - params.box_min) / params.cell),
        params.dims - vec3u(1u),
    );
    var rho = 0.0;
    for (var dz = -1i; dz <= 1i; dz++) {
        for (var dy = -1i; dy <= 1i; dy++) {
            for (var dx = -1i; dx <= 1i; dx++) {
                let coord = vec3i(base) + vec3i(dx, dy, dz);
                if any(coord < vec3i(0)) || any(coord >= vec3i(params.dims)) {
                    continue;
                }
                let c = (u32(coord.z) * params.dims.y + u32(coord.y)) * params.dims.x
                    + u32(coord.x);
                let end = starts[c] + counts[c];
                for (var k = starts[c]; k < end; k++) {
                    let r = distance(pos, positions[sorted[k]].xyz);
                    rho += params.mass * kernel(r, params.h);
                }
            }
        }
    }
    let lo = params.box_min + vec3f(params.cell);
    let hi = -lo;
    let inv_h = 1.0 / params.h;
    var fill = 0.0;
    fill += wall_density((pos.x - lo.x) * inv_h) + wall_density((hi.x - pos.x) * inv_h);
    fill += wall_density((pos.y - lo.y) * inv_h) + wall_density((hi.y - pos.y) * inv_h);
    fill += wall_density((pos.z - lo.z) * inv_h) + wall_density((hi.z - pos.z) * inv_h);
    density[id.x] = rho + params.rho0 * fill;
}

var<workgroup> red_sum: array<f32, 256>;
var<workgroup> red_max: array<f32, 256>;

// Compression error max(rho/rho0 - 1, 0): the free surface reads as
// deficiency and must not count. One workgroup strides the particles.
@compute @workgroup_size(256)
fn reduce_compression(@builtin(local_invocation_id) local: vec3u) {
    var s = 0.0;
    var m = 0.0;
    for (var i = local.x; i < params.count; i += 256u) {
        let c = max(density[i] / params.rho0 - 1.0, 0.0);
        s += c;
        m = max(m, c);
    }
    red_sum[local.x] = s;
    red_max[local.x] = m;
    workgroupBarrier();
    for (var off = 128u; off > 0u; off >>= 1u) {
        if local.x < off {
            red_sum[local.x] += red_sum[local.x + off];
            red_max[local.x] = max(red_max[local.x], red_max[local.x + off]);
        }
        workgroupBarrier();
    }
    if local.x == 0u {
        stats[0] = red_sum[0] / f32(params.count);
        stats[1] = red_max[0];
    }
}
