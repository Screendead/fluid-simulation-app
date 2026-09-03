// The stage-0 microbench: a counting sort into an index list, then SPH
// density over the 27 neighbour cells through that list, cubic-spline
// kernel. The shipped solver sorts the particle records themselves
// (sim_grid.wgsl); this keeps the seed order the bench validates
// against. Mirrors sim.rs::kernel and sim.rs::Grid exactly; a
// divergence is a bug.

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
// After scatter each cursor holds its cell's count; scatter zeroed the
// counts for the next rebuild.
@group(0) @binding(2) var<storage, read_write> cursors: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read> starts: array<u32>;
@group(0) @binding(4) var<storage, read_write> sorted: array<u32>;
@group(0) @binding(5) var<storage, read_write> density: array<f32>;
@group(0) @binding(6) var<storage, read_write> counts: array<atomic<u32>>;

fn cell_of(pos: vec3f) -> u32 {
    let coord = min(
        vec3u((pos - params.box_min) / params.cell),
        params.dims - vec3u(1u),
    );
    return (coord.z * params.dims.y + coord.y) * params.dims.x + coord.x;
}

@compute @workgroup_size(256)
fn count(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    atomicAdd(&counts[cell_of(positions[id.x].xyz)], 1u);
}

@compute @workgroup_size(256)
fn scatter(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let c = cell_of(positions[id.x].xyz);
    sorted[starts[c] + atomicAdd(&cursors[c], 1u)] = id.x;
    let cells = params.dims.x * params.dims.y * params.dims.z;
    for (var i = id.x; i < cells; i += params.count) {
        atomicStore(&counts[i], 0u);
    }
}

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
                let end = starts[c] + atomicLoad(&cursors[c]);
                for (var k = starts[c]; k < end; k++) {
                    let r = distance(pos, positions[sorted[k]].xyz);
                    rho += params.mass * kernel(r, params.h);
                }
            }
        }
    }
    density[id.x] = rho;
}
