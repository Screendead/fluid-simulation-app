// The liquid-glass surface: read the splatted field as a heightfield
// over a dazzle-painted back wall and run the real optics — Snell
// refraction through the local thickness, Schlick Fresnel against a
// gradient sky, Beer-Lambert absorption. The M4 record holds the
// model. Everything uniform across a frame — world up, the glint half
// vector and gain — arrives precomputed in the immediates; air pixels
// take the early return and pay one stripe lookup.

@group(0) @binding(0) var field: texture_2d<f32>;
@group(0) @binding(1) var field_sampler: sampler;

struct Optics {
    up: vec3f,
    field_settled: f32,
    extent: vec2f,
    slab_depth: f32,
    glint_gain: f32,
    h: vec3f,
}
var<immediate> optics: Optics;

// The liquid/air edge band, in field units.
const EDGE_LO: f32 = 0.8;
const EDGE_HI: f32 = 1.6;
const ETA: f32 = 1.0 / 1.33;
const F0: f32 = 0.02;
// Transmittance one slab depth of path down: red dies first, so the
// thin edge reads clear and the interior reads water.
const ABSORB: vec3f = vec3f(0.60, 0.25, 0.073);
const ZENITH: vec3f = vec3f(0.58, 0.72, 0.88);
const HORIZON: vec3f = vec3f(0.07, 0.10, 0.16);
const SUN_TINT: vec3f = vec3f(1.0, 0.98, 0.92);
const SUN_GLOSS: f32 = 700.0;

// Dazzle dials: the fan's sector count and centre, the two inks, and
// one row per sector — stripe direction, 1/period (per metre), phase.
// Entry 7 repeats entry 0: +pi and -pi are the same ray. The shipped
// rows came from fract(sin((k + o)*127.1 + 935.1)*43758.547), o in
// {0, 17, 41}, periods mapped to 0.022-0.05 m; edit rows freely.
const SECTORS: f32 = 7.0;
const CENTRE: vec2f = vec2f(0.05, 0.12);
const INK: vec3f = vec3f(0.02, 0.025, 0.035);
const PAPER: vec3f = vec3f(0.82, 0.84, 0.86);
const SECT = array<vec4f, 8>(
    vec4f(-0.29676, 0.954952, 40.774, 0.450765),
    vec4f(0.527222, 0.849727, 25.356, 0.272029),
    vec4f(-0.728458, 0.685091, 24.955, 0.202251),
    vec4f(-0.0281877, 0.999603, 20.608, 0.946907),
    vec4f(0.985455, 0.169935, 23.012, 0.620295),
    vec4f(0.943843, 0.330395, 21.462, 0.968535),
    vec4f(-0.496624, 0.867966, 21.65, 0.4454),
    vec4f(-0.29676, 0.954952, 40.774, 0.450765),
);

// One filtered stripe family per angular sector. The filter width is
// the analytic screen-space rate of the stripe coordinate — the L1
// width fwidth would report, exact on the wall plane — so the shader
// needs no derivatives and branches stay legal. Refraction reuses the
// wall rate: magnified stripes soften safely, compressed ones run a
// touch crisp.
fn dazzle(p: vec2f, px2w: vec2f) -> vec3f {
    let q = p - CENTRE;
    let k = i32(floor((atan2(q.y, q.x) * 0.15915494 + 0.5) * SECTORS));
    let row = SECT[k];
    let s = dot(p, row.xy) * row.z + row.w;
    let w = min((abs(row.x) * px2w.x + abs(row.y) * px2w.y) * row.z, 0.5);
    let lit = 1.0 - smoothstep(0.25 - w, 0.25 + w, abs(fract(s) - 0.5));
    return mix(INK, PAPER, lit);
}

struct FillVertex {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn fill(@builtin(vertex_index) v: u32) -> FillVertex {
    let xy = vec2f(f32((v << 1u) & 2u), f32(v & 2u));
    var out: FillVertex;
    out.clip = vec4f(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2f(xy.x, 1.0 - xy.y);
    return out;
}

// The decay draw: source factor zero, destination factor the keep
// fraction, so only the blend does the work.
@fragment
fn decay_frag(in: FillVertex) -> @location(0) vec4f {
    return vec4f(0.0);
}

@fragment
fn surface_frag(in: FillVertex) -> @location(0) vec4f {
    let d = textureSampleLevel(field, field_sampler, in.uv, 0.0).r;
    let a = smoothstep(EDGE_LO, EDGE_HI, d);

    // This pixel on the back wall, metres, y up.
    let p = vec2f(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0) * optics.extent;
    let px2w = optics.extent / (2.0 * vec2f(textureDimensions(field)));
    if (a <= 0.0) {
        return vec4f(dazzle(p, px2w), 1.0);
    }

    // Thickness from the calibrated field; the clamp stops an EMA
    // transient from throwing the refraction across the screen.
    let t = clamp(d / optics.field_settled, 0.0, 1.25) * optics.slab_depth;

    // Front-surface normal from the thickness gradient, central
    // differences one field texel apart. uv.y runs against world y.
    let texel = 1.0 / vec2f(textureDimensions(field));
    let dx = textureSampleLevel(field, field_sampler, in.uv + vec2f(texel.x, 0.0), 0.0).r
        - textureSampleLevel(field, field_sampler, in.uv - vec2f(texel.x, 0.0), 0.0).r;
    let dy = textureSampleLevel(field, field_sampler, in.uv + vec2f(0.0, texel.y), 0.0).r
        - textureSampleLevel(field, field_sampler, in.uv - vec2f(0.0, texel.y), 0.0).r;
    let scale = optics.slab_depth
        / (optics.field_settled * 4.0 * optics.extent * texel);
    let n = normalize(vec3f(-dx * scale.x, dy * scale.y, 1.0));

    // Snell into the water, through the thickness, onto the wall.
    let view = vec3f(0.0, 0.0, -1.0);
    let r = refract(view, n, ETA);
    let path = t / max(-r.z, 0.2);
    let through = dazzle(p + r.xy * path, px2w)
        * exp(-ABSORB * (path / optics.slab_depth));

    let fres = F0 + (1.0 - F0) * pow(1.0 - n.z, 5.0);
    let rr = reflect(view, n);
    let sky = mix(HORIZON, ZENITH, 0.5 + 0.5 * dot(rr, optics.up));
    let glint = pow(max(dot(n, optics.h), 0.0), SUN_GLOSS) * optics.glint_gain;

    var col = mix(through, sky, fres) + SUN_TINT * glint;
    if (a < 1.0) {
        col = mix(dazzle(p, px2w), col, a);
    }
    return vec4f(col, 1.0);
}
