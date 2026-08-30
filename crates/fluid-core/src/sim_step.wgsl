// One transport substep: the body force, then the walls. Pressure,
// viscosity, temperature and the counted CFL clamp enter with the later
// solver stages — nothing here reads a neighbour, so no CFL bound binds
// yet. Walls are inelastic because water does not bounce off glass.

struct Step {
    force: vec3f,
    dt: f32,
}

struct SimParams {
    box_min: vec3f,
    cell: f32,
    dims: vec3u,
    count: u32,
    h: f32,
    mass: f32,
    rho0: f32,
}

var<immediate> step: Step;

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read_write> positions: array<vec4f>;
@group(0) @binding(2) var<storage, read_write> velocities: array<vec4f>;

@compute @workgroup_size(256)
fn substep(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    var v = velocities[id.x].xyz + step.force * step.dt;
    var p = positions[id.x].xyz + v * step.dt;
    // params.box_min includes the guard cell; the wall sits one cell in.
    let lo = params.box_min + vec3f(params.cell);
    let hi = -lo;
    let clamped = clamp(p, lo, hi);
    v = select(v, vec3f(0.0), clamped != p);
    positions[id.x] = vec4f(clamped, 0.0);
    velocities[id.x] = vec4f(v, 0.0);
}
