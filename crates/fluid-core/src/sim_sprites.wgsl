// Sprites vertex-pulled from the 3D sim state; the slab projects
// orthographically, x and y to clip, z ignored until M4 wants depth.

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
@group(0) @binding(2) var<storage, read> velocities: array<vec4f>;

const CALM: vec3f = vec3f(0.05, 0.22, 0.55);
const LIVELY: vec3f = vec3f(0.72, 0.94, 1.0);
const FULL_SPEED: f32 = 0.9;

struct SpriteVertex {
    @builtin(position) clip: vec4f,
    @location(0) corner: vec2f,
    @location(1) colour: vec3f,
}

@vertex
fn sprite(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> SpriteVertex {
    let extent = -(params.box_min.xy + vec2f(params.cell));
    let radius = params.h * 0.5;
    let corner = vec2f(f32(v & 1u), f32(v >> 1u)) * 2.0 - 1.0;
    var out: SpriteVertex;
    out.clip = vec4f((positions[i].xy + corner * radius) / extent, 0.0, 1.0);
    out.corner = corner;
    out.colour = mix(
        CALM,
        LIVELY,
        clamp(length(velocities[i].xyz) / FULL_SPEED, 0.0, 1.0),
    );
    return out;
}

@fragment
fn glow(in: SpriteVertex) -> @location(0) vec4f {
    let falloff = max(1.0 - dot(in.corner, in.corner), 0.0);
    let a = falloff * falloff;
    return vec4f(in.colour * a, a);
}
