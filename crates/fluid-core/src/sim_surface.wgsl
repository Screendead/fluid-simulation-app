// The liquid-glass surface: read the splatted field as a heightfield
// over a dazzle-painted back wall and run the real optics — Snell
// refraction through the local thickness, Schlick Fresnel against a
// gradient sky, Beer-Lambert absorption. The M4 record holds the
// model. The light pins to world-up through the filtered body force,
// so the highlight slides with real tilt.

@group(0) @binding(0) var field: texture_2d<f32>;
@group(0) @binding(1) var field_sampler: sampler;

struct Optics {
    force: vec3f,
    field_settled: f32,
    extent: vec2f,
    slab_depth: f32,
}
var<immediate> optics: Optics;

// The liquid/air edge band, in field units.
const EDGE_LO: f32 = 0.8;
const EDGE_HI: f32 = 1.6;
const ETA: f32 = 1.0 / 1.33;
const F0: f32 = 0.02;
// Transmittance one slab depth down: red dies first, so the thin edge
// reads clear and the interior reads water.
const ABSORB: vec3f = vec3f(0.60, 0.25, 0.073);
const ZENITH: vec3f = vec3f(0.58, 0.72, 0.88);
const HORIZON: vec3f = vec3f(0.07, 0.10, 0.16);
// The sun outshines the sky by orders of magnitude; the two-percent
// Fresnel floor of it still reads as a hot glint.
const SUN: f32 = 60.0;
const SUN_TINT: vec3f = vec3f(1.0, 0.98, 0.92);
const SUN_GLOSS: f32 = 700.0;

// Dazzle dials: sector count, seed, stripe period range in metres,
// the sector fan's centre in metres, and the two inks.
const SECTORS: f32 = 7.0;
const SEED: f32 = 3.0;
const TWIST: f32 = 0.0;
const PERIOD_LO: f32 = 0.022;
const PERIOD_HI: f32 = 0.05;
const CENTRE: vec2f = vec2f(0.05, 0.12);
const INK: vec3f = vec3f(0.02, 0.025, 0.035);
const PAPER: vec3f = vec3f(0.82, 0.84, 0.86);

fn hash(k: f32) -> f32 {
    return fract(sin(k * 127.1 + SEED * 311.7) * 43758.547);
}

// One filtered stripe family per angular sector. fwidth runs on the
// refracted coordinate, so magnified stripes stay crisp and compressed
// ones fade to grey instead of aliasing.
fn dazzle(p: vec2f) -> vec3f {
    let q = p - CENTRE;
    let sector = floor((atan2(q.y, q.x) * 0.15915494 + 0.5) * SECTORS);
    let phi = hash(sector) * 3.1415927 + TWIST;
    let dir = vec2f(cos(phi), sin(phi));
    let s = dot(p, dir) / mix(PERIOD_LO, PERIOD_HI, hash(sector + 17.0))
        + hash(sector + 41.0);
    let w = min(fwidth(s), 0.5);
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
    let d = textureSample(field, field_sampler, in.uv).r;
    let a = smoothstep(EDGE_LO, EDGE_HI, d);

    // This pixel on the back wall, metres, y up.
    let p = vec2f(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0) * optics.extent;

    // Thickness from the calibrated field; the clamp stops an EMA
    // transient from throwing the refraction across the screen.
    let t = clamp(d / optics.field_settled, 0.0, 1.25) * optics.slab_depth;

    // Front-surface normal from the thickness gradient, central
    // differences one field texel apart. uv.y runs against world y.
    let texel = 1.0 / vec2f(textureDimensions(field));
    let dx = textureSample(field, field_sampler, in.uv + vec2f(texel.x, 0.0)).r
        - textureSample(field, field_sampler, in.uv - vec2f(texel.x, 0.0)).r;
    let dy = textureSample(field, field_sampler, in.uv + vec2f(0.0, texel.y)).r
        - textureSample(field, field_sampler, in.uv - vec2f(0.0, texel.y)).r;
    let scale = optics.slab_depth
        / (optics.field_settled * 4.0 * optics.extent * texel);
    let n = normalize(vec3f(-dx * scale.x, dy * scale.y, 1.0));

    // Snell into the water, through the thickness, onto the wall.
    let view = vec3f(0.0, 0.0, -1.0);
    let r = refract(view, n, ETA);
    let path = t / max(-r.z, 0.2);
    let through = dazzle(p + r.xy * path)
        * exp(-ABSORB * (path / optics.slab_depth));

    let fres = F0 + (1.0 - F0) * pow(1.0 - n.z, 5.0);

    // World up from the filtered force; under a metre per second
    // squared the box is near free fall and the sky holds still.
    let g = optics.force;
    let up = select(
        vec3f(0.0, 0.0, 1.0),
        -g / max(length(g), 1e-6),
        length(g) > 1.0,
    );
    let rr = reflect(view, n);
    let sky = mix(HORIZON, ZENITH, 0.5 + 0.5 * dot(rr, up));

    // The light hangs over the viewer's shoulder, pinned to world up.
    // An orthographic view under a directional light is degenerate on
    // flat water — every pixel hits the glint angle at once, and
    // face-up is exactly that pose. The viewer's own head shades it:
    // the sun fades out as up aligns with the view axis. All the way
    // out: any floor lets the lobe resolve the particle lattice on a
    // one-layer sheet as a honeycomb of florets.
    let light = normalize(up + vec3f(0.0, 0.0, 0.8));
    // Face down, light meets view head on and h degenerates to zero;
    // the guarded divide keeps it a zero glint instead of a NaN frame.
    let hv = light - view;
    let h = hv / max(length(hv), 1e-5);
    let glint = pow(max(dot(n, h), 0.0), SUN_GLOSS)
        * (F0 + (1.0 - F0) * pow(1.0 - dot(h, -view), 5.0))
        * (1.0 - smoothstep(0.85, 0.98, up.z));

    let water = mix(through, sky, fres) + SUN_TINT * (SUN * glint);
    return vec4f(mix(dazzle(p), water, a), 1.0);
}
