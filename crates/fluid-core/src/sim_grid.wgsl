// Counting-sort neighbour grid, passes clear, count and scatter. The cell
// arithmetic mirrors sim.rs::Grid exactly; a divergence is a bug.

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
@group(0) @binding(2) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read> starts: array<u32>;
@group(0) @binding(4) var<storage, read_write> cursors: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> sorted: array<u32>;

fn cell_of(pos: vec3f) -> u32 {
    let coord = min(
        vec3u((pos - params.box_min) / params.cell),
        params.dims - vec3u(1u),
    );
    return (coord.z * params.dims.y + coord.y) * params.dims.x + coord.x;
}

@compute @workgroup_size(256)
fn clear_counts(@builtin(global_invocation_id) id: vec3u) {
    if id.x < arrayLength(&counts) {
        atomicStore(&counts[id.x], 0u);
    }
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
}
