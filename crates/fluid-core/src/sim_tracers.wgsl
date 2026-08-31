// The visual layer: massless tracers advected through the solver's
// velocity field, drawn as single-pixel points. Tracers never touch
// the physics — the field is splatted to the neighbour grid once a
// frame and sampled trilinearly, so half a million tracers cost less
// than one solver sweep.

struct SimParams {
    box_min: vec3f,
    cell: f32,
    dims: vec3u,
    count: u32,
    h: f32,
    mass: f32,
    rho0: f32,
}

struct Step {
    force: vec3f,
    dt: f32,
    v_clamp: f32,
    seed: u32,
}

var<immediate> step: Step;

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read> positions: array<vec4f>;
@group(0) @binding(2) var<storage, read> velocities: array<vec4f>;
@group(0) @binding(3) var<storage, read_write> vel_grid: array<atomic<i32>>;
@group(0) @binding(4) var<storage, read_write> tracers: array<vec4f>;

// Velocities land in the grid as 16.16 fixed point; f32 storage has no
// atomic add.
const FIXED: f32 = 65536.0;

// Recycling time constant: the expected life of a tracer before it
// respawns at a solver particle.
const TAU: f32 = 3.0;

fn pcg(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

fn cell_count() -> u32 {
    return params.dims.x * params.dims.y * params.dims.z;
}

@compute @workgroup_size(256)
fn clear_vel(@builtin(global_invocation_id) id: vec3u) {
    if id.x < cell_count() * 4u {
        atomicStore(&vel_grid[id.x], 0i);
    }
}

@compute @workgroup_size(256)
fn splat(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let pos = positions[id.x].xyz;
    let coord = min(
        vec3u((pos - params.box_min) / params.cell),
        params.dims - vec3u(1u),
    );
    let c = ((coord.z * params.dims.y + coord.y) * params.dims.x + coord.x) * 4u;
    let v = velocities[id.x].xyz;
    atomicAdd(&vel_grid[c], i32(v.x * FIXED));
    atomicAdd(&vel_grid[c + 1u], i32(v.y * FIXED));
    atomicAdd(&vel_grid[c + 2u], i32(v.z * FIXED));
    atomicAdd(&vel_grid[c + 3u], 1i);
}

@compute @workgroup_size(256)
fn advect(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&tracers) {
        return;
    }
    // A sampled field is compressible where the flow is not, so a
    // passive cloud collapses onto attractors over minutes and strands
    // on walls the fluid has left. Respawning a random fraction at
    // solver particles relaxes the cloud back to the true fluid with
    // time constant TAU.
    let r0 = pcg(id.x ^ pcg(step.seed));
    if f32(r0) / 4294967295.0 < step.dt / TAU {
        let r1 = pcg(r0);
        let r2 = pcg(r1);
        let r3 = pcg(r2);
        let r4 = pcg(r3);
        let jitter = (vec3f(vec3u(r2, r3, r4) >> vec3u(20u)) / 4096.0 - vec3f(0.5))
            * (params.cell * 0.5);
        let j = r1 % params.count;
        let lo = params.box_min + vec3f(params.cell);
        let p = clamp(positions[j].xyz + jitter, lo, -lo);
        tracers[id.x] = vec4f(p, length(velocities[j].xyz));
        return;
    }
    var pos = tracers[id.x].xyz;
    // Trilinear over the eight nearest cell centres.
    let gp = (pos - params.box_min) / params.cell - vec3f(0.5);
    let base = vec3u(clamp(vec3i(floor(gp)), vec3i(0), vec3i(params.dims) - vec3i(2)));
    let f = clamp(gp - vec3f(base), vec3f(0.0), vec3f(1.0));
    var moment = vec3f(0.0);
    var weight_count = 0.0;
    for (var k = 0u; k < 8u; k++) {
        let o = vec3u(k & 1u, (k >> 1u) & 1u, (k >> 2u) & 1u);
        let w = mix(vec3f(1.0) - f, f, vec3f(o));
        let weight = w.x * w.y * w.z;
        let coord = base + o;
        let c = ((coord.z * params.dims.y + coord.y) * params.dims.x + coord.x) * 4u;
        moment += weight
            * vec3f(
                f32(atomicLoad(&vel_grid[c])),
                f32(atomicLoad(&vel_grid[c + 1u])),
                f32(atomicLoad(&vel_grid[c + 2u])),
            );
        weight_count += weight * f32(atomicLoad(&vel_grid[c + 3u]));
    }
    var v = vec3f(0.0);
    if weight_count > 0.0 {
        v = moment / (weight_count * FIXED);
    }
    // An unsampled tracer sits outside the fluid and waits; the field
    // carries it again when the fluid returns.
    pos += v * step.dt;
    let lo = params.box_min + vec3f(params.cell);
    pos = clamp(pos, lo, -lo);
    tracers[id.x] = vec4f(pos, length(v));
}
