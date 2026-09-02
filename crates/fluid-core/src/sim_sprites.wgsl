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
// w carries the acceleration the solver's integrate step measured.
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
    // z carries the particle's place on the ramp, 0 to 1. Neither pass
    // has a depth attachment, so clip z is an interpolant already paid
    // for, and a varying of its own is not cheap: one flat vec3 on the
    // disc draw cost 490 us a frame at 1,620 discs (reference device,
    // 2026-09-02). Every vertex of a quad carries the same value, so
    // the fragment reads it back exactly.
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
    var lens = 0.0;
    if paint.high.w > 0.0 {
        lens = lens_at(i);
    }
    out.clip = vec4f((positions[i].xy + corner * params.h * 1.5) / extent, lens, 1.0);
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
    let w = falloff * falloff * 0.5;
    // The second channel is the same splat weighted by the lens, so
    // the surface recovers a kernel-weighted mean by dividing. Both
    // channels decay together, which leaves the ratio alone.
    return vec4f(w, w * in.clip.z, 0.0, 0.0);
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

// How the flat looks are coloured, shared by the disc draw and the
// body splat. pack_paint in render.rs is the packer.
struct Paint {
    // Linear light. low.w is one for either flat look and zero for the
    // glass; high.w is one only when the ramp is on.
    low: vec4f,
    high: vec4f,
    lens: u32,
    // The ends of the lens's ramp, in the lens's own units.
    lo: f32,
    hi: f32,
    // Metres: three device pixels of the drawable, so the smallest
    // disc is a dot and never one pixel. The splat ignores it.
    r_min: f32,
}
var<immediate> paint: Paint;

@group(0) @binding(4) var<storage, read> density: array<f32>;
@group(0) @binding(5) var<storage, read> pressure: array<f32>;
@group(0) @binding(6) var<storage, read> temperature: array<f32>;

// The box's starting temperature, kelvin: AMBIENT_TEMPERATURE in
// sim.rs, and the 20 C the lab constants in sim_solve.wgsl are quoted
// at.
const AMBIENT: f32 = 293.15;

// Where particle i sits on the ramp, 0 to 1. Called only where the
// ramp is on: the buffer loads are the whole cost, and the glass look
// pays none of them. Lens::code in render.rs numbers the cases.
fn lens_at(i: u32) -> f32 {
    var m = 0.0;
    switch paint.lens {
        case 0u: {
            m = length(velocities[i].xyz);
        }
        case 1u: {
            m = velocities[i].w;
        }
        case 2u: {
            m = pressure[i];
        }
        case 3u: {
            m = density[i];
        }
        default: {
            m = temperature[i] - AMBIENT;
        }
    }
    return clamp((m - paint.lo) / (paint.hi - paint.lo), 0.0, 1.0);
}

@vertex
fn disc(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> BodyVertex {
    let extent = -(params.box_min.xy + vec2f(params.cell));
    let corner = vec2f(f32(v & 1u), f32(v >> 1u)) * 2.0 - 1.0;
    let crowd = clamp(
        (density[i] / params.rho0 - LONE_RHO) / (BODY_RHO - LONE_RHO),
        0.0,
        1.0,
    );
    let radius = mix(paint.r_min, params.h * DISC_RADIUS, crowd);
    var lens = 0.0;
    if paint.high.w > 0.0 {
        lens = lens_at(i);
    }
    var out: BodyVertex;
    out.clip = vec4f((positions[i].xy + corner * radius) / extent, lens, 1.0);
    out.corner = corner;
    return out;
}

@fragment
fn disc_frag(in: BodyVertex) -> @location(0) vec4f {
    if dot(in.corner, in.corner) > 1.0 {
        discard;
    }
    // With no ramp the high colour is zero and so is the lens, so the
    // mix is the low colour and needs no branch of its own.
    return vec4f(mix(paint.low.rgb, paint.high.rgb, in.clip.z), 1.0);
}
