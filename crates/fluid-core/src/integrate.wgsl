// Semi-implicit Euler under the in-plane body force, then reflection off
// the box walls. Tunables are per-second rates so dt stays out of them.

struct Particle {
    pos: vec2f,
    vel: vec2f,
}

struct Params {
    force: vec2f,
    dt: f32,
    radius: f32,
    extent: vec2f,
    count: u32,
    _pad: u32,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: Params;

const DRAG_PER_SECOND: f32 = 0.6;
const RESTITUTION: f32 = 0.55;

@compute @workgroup_size(64)
fn integrate(@builtin(global_invocation_id) id: vec3u) {
    let i = id.x;
    if i >= params.count {
        return;
    }
    var p = particles[i];
    p.vel = (p.vel + params.force * params.dt) * (1.0 - DRAG_PER_SECOND * params.dt);
    p.pos += p.vel * params.dt;
    let limit = params.extent - vec2f(params.radius);
    if p.pos.x < -limit.x { p.pos.x = -limit.x; p.vel.x = abs(p.vel.x) * RESTITUTION; }
    if p.pos.x > limit.x { p.pos.x = limit.x; p.vel.x = -abs(p.vel.x) * RESTITUTION; }
    if p.pos.y < -limit.y { p.pos.y = -limit.y; p.vel.y = abs(p.vel.y) * RESTITUTION; }
    if p.pos.y > limit.y { p.pos.y = limit.y; p.vel.y = -abs(p.vel.y) * RESTITUTION; }
    particles[i] = p;
}
