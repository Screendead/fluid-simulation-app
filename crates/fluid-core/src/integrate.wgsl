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

// Without inter-particle forces (M3), identical dynamics collapse every
// particle onto one point: same force, same drag, same wall clamp. Each
// particle therefore gets a hash-derived personal rest offset, restitution
// and a scaled force (0.9 to 1.1) so a falling stream stretches out. A
// rotated force is rejected: with the force nearly wall-parallel a fixed
// tilt sends a subset of particles up the wall into the wrong corner
// (observed on the device, 2026-08-30). The
// offsets sample a quarter disc in physical units, densest at the wall: a
// per-axis inset settles into a screen-aspect rectangle (observed on the
// device, 2026-08-30), a disc settles into a rounded pool. The disc radius
// is a fraction of the box's short half-extent.
const POOL_RADIUS_FRACTION: f32 = 0.5;

fn hash(x: u32) -> u32 {
    var h = x * 0x9E3779B9u;
    h ^= h >> 16u;
    h = h * 0x85EBCA6Bu;
    return h ^ (h >> 13u);
}

fn unit(h: u32) -> f32 {
    return f32(h) / 4294967295.0;
}

@compute @workgroup_size(64)
fn integrate(@builtin(global_invocation_id) id: vec3u) {
    let i = id.x;
    if i >= params.count {
        return;
    }
    let h1 = unit(hash(i * 3u + 1u));
    let h2 = unit(hash(i * 3u + 2u));
    let h3 = unit(hash(i * 3u + 3u));
    let force = params.force * (0.9 + 0.2 * h3);
    let restitution = 0.3 + 0.4 * h3;
    let pool = POOL_RADIUS_FRACTION * min(params.extent.x, params.extent.y);
    let theta = h2 * 1.5707963;
    let limit = params.extent - vec2f(params.radius)
        - pool * h1 * vec2f(cos(theta), sin(theta));

    var p = particles[i];
    p.vel = (p.vel + force * params.dt) * (1.0 - DRAG_PER_SECOND * params.dt);
    p.pos += p.vel * params.dt;
    if p.pos.x < -limit.x { p.pos.x = -limit.x; p.vel.x = abs(p.vel.x) * restitution; }
    if p.pos.x > limit.x { p.pos.x = limit.x; p.vel.x = -abs(p.vel.x) * restitution; }
    if p.pos.y < -limit.y { p.pos.y = -limit.y; p.vel.y = abs(p.vel.y) * restitution; }
    if p.pos.y > limit.y { p.pos.y = limit.y; p.vel.y = -abs(p.vel.y) * restitution; }
    particles[i] = p;
}
