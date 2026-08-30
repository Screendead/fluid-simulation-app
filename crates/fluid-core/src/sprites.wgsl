// Soft sprites vertex-pulled from the particle buffer: four strip corners
// per instance, no vertex buffer.

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

@group(0) @binding(0) var<storage, read> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: Params;

// The colour floor keeps a resting particle visible; full speed is about
// the free-fall speed across the screen's height.
const CALM: vec3f = vec3f(0.05, 0.22, 0.55);
const LIVELY: vec3f = vec3f(0.72, 0.94, 1.0);
const FULL_SPEED: f32 = 1.7;

struct SpriteVertex {
    @builtin(position) clip: vec4f,
    @location(0) corner: vec2f,
    @location(1) colour: vec3f,
}

@vertex
fn sprite(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> SpriteVertex {
    let p = particles[i];
    let corner = vec2f(f32(v & 1u), f32(v >> 1u)) * 2.0 - 1.0;
    var out: SpriteVertex;
    out.clip = vec4f((p.pos + corner * params.radius) / params.extent, 0.0, 1.0);
    out.corner = corner;
    out.colour = mix(CALM, LIVELY, clamp(length(p.vel) / FULL_SPEED, 0.0, 1.0));
    return out;
}

@fragment
fn glow(in: SpriteVertex) -> @location(0) vec4f {
    let falloff = max(1.0 - dot(in.corner, in.corner), 0.0);
    let a = falloff * falloff;
    return vec4f(in.colour * a, a);
}
