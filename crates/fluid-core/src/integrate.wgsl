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
// particle therefore gets a hash-derived personal wall inset (squared, so
// density peaks at the wall and a pile reads as a pool), restitution, and
// a slightly tilted, scaled force so a falling stream fans out.
const MAX_INSET: f32 = 0.35;

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
    let angle = (h1 - 0.5) * 0.25;
    let force = mat2x2f(vec2f(cos(angle), sin(angle)), vec2f(-sin(angle), cos(angle)))
        * params.force * (0.9 + 0.2 * h2);
    let restitution = 0.3 + 0.4 * h3;
    let limit = params.extent - vec2f(params.radius)
        - params.extent * MAX_INSET * vec2f(h1 * h1, h2 * h2);

    var p = particles[i];
    p.vel = (p.vel + force * params.dt) * (1.0 - DRAG_PER_SECOND * params.dt);
    p.pos += p.vel * params.dt;
    if p.pos.x < -limit.x { p.pos.x = -limit.x; p.vel.x = abs(p.vel.x) * restitution; }
    if p.pos.x > limit.x { p.pos.x = limit.x; p.vel.x = -abs(p.vel.x) * restitution; }
    if p.pos.y < -limit.y { p.pos.y = -limit.y; p.vel.y = abs(p.vel.y) * restitution; }
    if p.pos.y > limit.y { p.pos.y = limit.y; p.vel.y = -abs(p.vel.y) * restitution; }
    particles[i] = p;
}
