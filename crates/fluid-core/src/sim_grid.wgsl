// Counting-sort neighbour grid, passes count and scatter. The cell
// arithmetic mirrors sim.rs::Grid exactly; a divergence is a bug.
//
// The particles live in two sets. The resting set (the plain names)
// is what every per-frame reader binds; the working set is the same
// five records laid out in cell order for one substep's sweeps.
// scatter copies resting into working; integrate in sim_solve.wgsl
// writes the substep's result back to resting at the same slot, so
// the resting set is in cell order too once a substep has run.

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
@group(0) @binding(5) var<storage, read> velocities: array<vec4f>;
@group(0) @binding(6) var<storage, read> prev_vel: array<vec4f>;
@group(0) @binding(7) var<storage, read> prev_pressure: array<f32>;
@group(0) @binding(8) var<storage, read> temperature: array<f32>;
@group(0) @binding(9) var<storage, read_write> work_positions: array<vec4f>;
@group(0) @binding(10) var<storage, read_write> work_velocities: array<vec4f>;
@group(0) @binding(11) var<storage, read_write> work_prev_vel: array<vec4f>;
@group(0) @binding(12) var<storage, read_write> work_prev_pressure: array<f32>;
@group(0) @binding(13) var<storage, read_write> work_temperature: array<f32>;

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
    let k = starts[c] + atomicAdd(&cursors[c], 1u);
    work_positions[k] = positions[id.x];
    work_velocities[k] = velocities[id.x];
    work_prev_vel[k] = prev_vel[id.x];
    work_prev_pressure[k] = prev_pressure[id.x];
    work_temperature[k] = temperature[id.x];
    // The scan has consumed the counts and the sweeps read cell ends
    // from starts, so the counts die here: zero them for the next
    // rebuild instead of a clear dispatch of their own. Buffers start
    // zeroed, which covers the first rebuild.
    let cells = params.dims.x * params.dims.y * params.dims.z;
    for (var i = id.x; i < cells; i += params.count) {
        atomicStore(&counts[i], 0u);
    }
}
