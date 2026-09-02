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
// The charge below which a tracer draws nothing: resting water shows
// no dots, and the idle gate sleeps only under this.
const CHARGE_GATE: f32 = 0.05;

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

@group(0) @binding(3) var<storage, read> tracers: array<vec2u>;

struct PointVertex {
    @builtin(position) clip: vec4f,
    @location(0) colour: vec3f,
}

// The one-pixel tracer draw; the tracer buffer is read-only here so the
// vertex stage needs no writable-storage feature.
@vertex
fn point(@builtin(vertex_index) i: u32) -> PointVertex {
    let t = tracers[i];
    var out: PointVertex;
    // Brightness rides on the charge: a resting dot vanishes instead
    // of speckling the body, and fast water glints. The square root
    // lifts the gentle end so slow strands still read as threads
    // (Jack's dial, 2026-09-01); full speed is unchanged.
    let s = clamp((unpack2x16float(t.y).y - CHARGE_GATE) / FULL_SPEED, 0.0, 1.0);
    out.colour = mix(CALM, LIVELY, s) * (sqrt(s) * 0.9);
    // The record quantises over the box this draw projects with, so the
    // unorm pair is already clip space; the packed z goes unread. A
    // chargeless dot adds nothing under the additive blend, so it parks
    // outside the clip volume and the rasteriser never sees it.
    out.clip = select(
        vec4f(unpack2x16unorm(t.x) * 2.0 - vec2f(1.0), 0.0, 1.0),
        vec4f(2.0, 2.0, 0.0, 1.0),
        s <= 0.0,
    );
    return out;
}

@fragment
fn dot_frag(in: PointVertex) -> @location(0) vec4f {
    return vec4f(in.colour, 0.0);
}

// The liquid body: each solver particle splats its kernel footprint
// into the half-resolution field the surface pass thresholds.
struct BodyVertex {
    @builtin(position) clip: vec4f,
    @location(0) corner: vec2f,
}

@vertex
fn body(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> BodyVertex {
    let extent = -(params.box_min.xy + vec2f(params.cell));
    let corner = vec2f(f32(v & 1u), f32(v >> 1u)) * 2.0 - 1.0;
    var out: BodyVertex;
    // 1.5 h, or the flat pose fails: gravity into the glass spreads
    // the fluid one particle deep, in-plane neighbours sit millimetres
    // apart, and footprints of radius h leave holes between particles.
    out.clip = vec4f((positions[i].xy + corner * params.h * 1.5) / extent, 0.0, 1.0);
    out.corner = corner;
    return out;
}

@fragment
fn weight(in: BodyVertex) -> @location(0) vec4f {
    let falloff = max(1.0 - dot(in.corner, in.corner), 0.0);
    // 0.5 mostly undoes the wider radius's area gain (exact would be
    // 0.44), held a little high so a few-particle droplet still clears
    // the edge threshold. The pass scales by 1 - keep through the
    // blend constant, the exact complement of the decay draw, so the
    // field's steady state is the raw splat at every keep.
    return vec4f(falloff * falloff * 0.5, 0.0, 0.0, 0.0);
}

// The particle view (M5 record): the water is its own particles, each
// a disc of the colour on black, and no thickness field is built at
// all. The radius is in h, so a disc holds the same fraction of the
// particle spacing at every scale of the ladder.
//
// Jack, 2026-09-02: a disc keeps its full size inside a body of water
// and shrinks as its neighbourhood thins, hardest when it is alone,
// and never below the pixel floor the immediates carry. Density is
// the measure of a neighbourhood, and the solver already has it: a
// settled slab reads 0.99 to 1.00 of rest density through the body
// and its free surface, its outermost fringe 0.67, a touching pair
// 0.26, and a lone particle 0.20, which is its own kernel weight
// (measured 2026-09-02, the_settled_body_keeps_its_discs).
const DISC_RADIUS: f32 = 0.5;
const BODY_RHO: f32 = 0.65;
const LONE_RHO: f32 = 0.25;

struct DiscLook {
    water: vec4f,
    // Metres: three device pixels of the drawable, so the smallest
    // disc is a dot and never one pixel.
    r_min: f32,
}
var<immediate> look: DiscLook;

@group(0) @binding(4) var<storage, read> density: array<f32>;

@vertex
fn disc(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> BodyVertex {
    let extent = -(params.box_min.xy + vec2f(params.cell));
    let corner = vec2f(f32(v & 1u), f32(v >> 1u)) * 2.0 - 1.0;
    let crowd = clamp(
        (density[i] / params.rho0 - LONE_RHO) / (BODY_RHO - LONE_RHO),
        0.0,
        1.0,
    );
    let radius = mix(look.r_min, params.h * DISC_RADIUS, crowd);
    var out: BodyVertex;
    out.clip = vec4f((positions[i].xy + corner * radius) / extent, 0.0, 1.0);
    out.corner = corner;
    return out;
}

@fragment
fn disc_frag(in: BodyVertex) -> @location(0) vec4f {
    if dot(in.corner, in.corner) > 1.0 {
        discard;
    }
    return vec4f(look.water.rgb, 1.0);
}
