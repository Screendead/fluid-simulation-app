// The liquid-glass surface: read the splatted field as a heightfield
// over a dazzle-painted back wall and run the real optics — Snell
// refraction through the local thickness, Schlick Fresnel against a
// gradient sky, Beer-Lambert absorption. The M4 record holds the
// model. Everything uniform across a frame — world up, the glint half
// vector and gain — arrives precomputed in the immediates; air pixels
// take the early return and pay one stripe lookup, and the flat look
// returns before the wall.

// The fill reads the filtered field: blurred thickness and its raw
// first and second texel differences, one sample a pixel. field_filter
// writes it from the raw splat.
@group(0) @binding(0) var field: texture_2d<f32>;
@group(0) @binding(1) var field_sampler: sampler;
@group(0) @binding(2) var filtered: texture_storage_2d<rgba16float, write>;
// The raw splat, which the fill also binds: r the thickness, g that
// thickness weighted by the lens. The flat look reads the ratio
// straight, unblurred — a kernel-weighted mean over a 1.5 h footprint
// is already smooth where the body is, and the blur exists for the
// caustics, which the flat look never runs.
@group(0) @binding(3) var splat: texture_2d<f32>;
// The direction lens's own field: the kernel-weighted sum of the unit
// heading vectors, which body_flow in sim_sprites.wgsl writes and only
// that lens fills. An angle has no mean across the seam at half a
// turn; the sum of the unit vectors has one everywhere.
@group(0) @binding(4) var flow: texture_2d<f32>;

const TAU: f32 = 6.2831855;

// Both mirrored from sim_sprites.wgsl, which paints the same wheel on
// the discs; a divergence is a bug.
// Gamma two, not sRGB's exact curve. The wheel's hues are chosen
// here rather than picked by the user, so the curve only has to look
// like a rainbow — and the exact one is three pows a fragment, which
// cost 600 microseconds a frame on the disc draw (reference device,
// 2026-09-02, 3,512 against 2,912). A picked colour still goes
// through the exact curve, in `linear` in render.rs.
fn to_linear(c: vec3f) -> vec3f {
    return c * c;
}

fn hue_colour(h: f32) -> vec3f {
    let k = abs(fract(h + vec3f(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
    return to_linear(clamp(k - 1.0, vec3f(0.0), vec3f(1.0)));
}

struct Optics {
    up: vec3f,
    field_settled: f32,
    extent: vec2f,
    slab_depth: f32,
    glint_gain: f32,
    h: vec3f,
    // The flat look's two colours in linear light. flat.w is one for
    // either flat look and zero for the glass. high.w says how the
    // body is painted: 0 the low colour alone, 1 the ramp from low to
    // high across the lens the splat normalised into the field's g
    // channel, 2 the direction wheel over that same channel.
    flat: vec4f,
    high: vec4f,
}
var<immediate> optics: Optics;

// The liquid/air edge band, in settled thicknesses: 0.8 and 1.6 field
// units at the shipped spacing, and the same water at every scale.
const EDGE_LO: f32 = 0.8 / 5.3;
const EDGE_HI: f32 = 1.6 / 5.3;
const ETA: f32 = 1.0 / 1.33;
const F0: f32 = 0.02;
// Transmittance one slab depth of path down: red dies first, so the
// thin edge reads clear and the interior reads water.
const ABSORB: vec3f = vec3f(0.60, 0.25, 0.073);
const ZENITH: vec3f = vec3f(0.58, 0.72, 0.88);
const HORIZON: vec3f = vec3f(0.07, 0.10, 0.16);
const SUN_TINT: vec3f = vec3f(1.0, 0.98, 0.92);
const SUN_GLOSS: f32 = 700.0;
// Caustics: the reciprocal Jacobian of the refraction map, linearised
// — the same map the wall sample takes, so brightness and stripe
// stretch stay consistent: compressed stripes brighten, magnified
// ones darken. CAUSTIC scales the physical 1 - eta term; the floor is
// the glass-edge shadow dial, judged at the waterline.
const CAUSTIC: f32 = 0.35;
const CAUSTIC_FLOOR: f32 = 0.5;
const CAUSTIC_CEIL: f32 = 2.0;
// Frost: stripe blur per slab depth of path, metres. The wall reads
// milky through deep water and stays crisp through the thin edge.
const FROST: f32 = 0.007;

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
fn dazzle(p: vec2f, px2w: vec2f, blur: f32) -> vec3f {
    let q = p - CENTRE;
    let k = i32(floor((atan2(q.y, q.x) * 0.15915494 + 0.5) * SECTORS));
    let row = SECT[k];
    let s = dot(p, row.xy) * row.z + row.w;
    let w = min((abs(row.x) * px2w.x + abs(row.y) * px2w.y + blur) * row.z, 0.5);
    let lit = 1.0 - smoothstep(0.25 - w, 0.25 + w, abs(fract(s) - 0.5));
    return mix(INK, PAPER, lit);
}

// The optics read the field through a 7-tap separable Gaussian: a
// wavelength filter, not cosmetics. Particle-footprint ripple lives
// near the inter-particle spacing and its curvature saturates the
// caustics; waves live at ten times that and pass through. The
// flicker meter demanded it, as the record said it might.
const BLUR = array<f32, 4>(0.3125, 0.234375, 0.09375, 0.015625);

// One workgroup filters a 16x16 tile: the raw field with a four-texel
// apron lands in workgroup memory, the rows blur, the columns blur,
// and the tile's centred differences come off the blurred
// neighbours. Loads clamp to the edge texel; the sampler clamped the
// blurred edge instead, a difference confined to the outermost texel.
// The fill's bilinear sample of these per-texel differences equals
// the differences of bilinear samples it took before, so the optics
// are unchanged up to the half-float store.
const TILE: u32 = 16u;
const APRON: u32 = 4u;
const SPAN: u32 = 24u;
const INNER: u32 = 18u;
var<workgroup> raw: array<f32, SPAN * SPAN>;
var<workgroup> rows: array<f32, SPAN * INNER>;

@compute @workgroup_size(16, 16)
fn field_filter(@builtin(workgroup_id) wg: vec3u, @builtin(local_invocation_id) l: vec3u) {
    let dims = vec2i(textureDimensions(field));
    let origin = vec2i(wg.xy) * i32(TILE) - i32(APRON);
    let lin = l.y * TILE + l.x;
    for (var i = lin; i < SPAN * SPAN; i += TILE * TILE) {
        let c = origin + vec2i(i32(i % SPAN), i32(i / SPAN));
        raw[i] = textureLoad(field, clamp(c, vec2i(0), dims - 1), 0).r;
    }
    workgroupBarrier();
    for (var i = lin; i < SPAN * INNER; i += TILE * TILE) {
        let base = (i / INNER) * SPAN + i % INNER + 3u;
        var acc = raw[base] * BLUR[0];
        for (var k = 1u; k <= 3u; k++) {
            acc += (raw[base + k] + raw[base - k]) * BLUR[k];
        }
        rows[i] = acc;
    }
    workgroupBarrier();
    for (var i = lin; i < INNER * INNER; i += TILE * TILE) {
        let base = i + 3u * INNER;
        var acc = rows[base] * BLUR[0];
        for (var k = 1u; k <= 3u; k++) {
            acc += (rows[base + k * INNER] + rows[base - k * INNER]) * BLUR[k];
        }
        raw[i] = acc;
    }
    workgroupBarrier();
    let centre = (l.y + 1u) * INNER + l.x + 1u;
    let d = raw[centre];
    let xp = raw[centre + 1u];
    let xm = raw[centre - 1u];
    let yp = raw[centre + INNER];
    let ym = raw[centre - INNER];
    let texel = vec2i(wg.xy) * i32(TILE) + vec2i(l.xy);
    if all(texel < dims) {
        // Second differences stay raw like the first: over the device's
        // 0.9 mm texel the Laplacian itself (1/step² ≈ 1.3e6) overflows
        // the half-float store at the waterline. The fill divides by
        // step.x²; the y term carries the texel aspect.
        let step = 2.0 * optics.extent / vec2f(dims);
        let aspect = (step.x * step.x) / (step.y * step.y);
        let lap = (xp + xm - 2.0 * d) + (yp + ym - 2.0 * d) * aspect;
        textureStore(filtered, texel, vec4f(d, xp - xm, ym - yp, lap));
    }
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

// The flat look: the water where the thickness crosses the band's
// midpoint, black everywhere else, and a hard edge between them
// (Jack, 2026-09-02). One chosen colour, or the ramp across the body.
// The particle view skips this pass entirely.
//
// `wheel` arrives as a literal from each entry point, so the branch
// folds away at compile time and the wheel's hue arithmetic never
// enters the shader the glass and the ramp share. Left as a runtime
// branch it cost the flat look 230 us a frame on a pixel that never
// took it: 3,332 microseconds before the wheel existed, 3,560 with
// the branch, 3,321 with it folded (reference device, 2026-09-02,
// back to back on a still desk).
fn flat_look(uv: vec2f, rel: f32, wheel: bool) -> vec4f {
    let water = rel >= 0.5 * (EDGE_LO + EDGE_HI);
    var colour = optics.flat.rgb;
    if (optics.high.w > 0.0) {
        // The threshold above is what makes the divide safe: no pixel
        // reaches here with a thickness near zero.
        let s = textureSampleLevel(splat, field_sampler, uv, 0.0);
        let t = s.g / max(s.r, 1e-6);
        if (wheel) {
            let d = textureSampleLevel(flow, field_sampler, uv, 0.0).rg;
            // The wheel takes over as the square, as it does on the
            // discs in sim_sprites.wgsl.
            colour = mix(optics.flat.rgb, hue_colour(atan2(d.y, d.x) / TAU + 0.5), t * t);
        } else {
            colour = mix(optics.flat.rgb, optics.high.rgb, t);
        }
    }
    return vec4f(select(vec3f(0.0), colour, water), 1.0);
}

// The wheel's own fill. Bound only by a flat look with the direction
// lens on, so it takes the flat branch without testing for it.
@fragment
fn surface_wheel_frag(in: FillVertex) -> @location(0) vec4f {
    let f = textureSampleLevel(field, field_sampler, in.uv, 0.0);
    return flat_look(in.uv, f.r / optics.field_settled, true);
}

@fragment
fn surface_frag(in: FillVertex) -> @location(0) vec4f {
    let f = textureSampleLevel(field, field_sampler, in.uv, 0.0);
    let rel = f.r / optics.field_settled;
    let a = smoothstep(EDGE_LO, EDGE_HI, rel);

    if (optics.flat.w > 0.0) {
        return flat_look(in.uv, rel, false);
    }

    // This pixel on the back wall, metres, y up.
    let p = vec2f(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0) * optics.extent;
    let px2w = optics.extent / (2.0 * vec2f(textureDimensions(field)));
    if (a <= 0.0) {
        return vec4f(dazzle(p, px2w, 0.0), 1.0);
    }

    // Thickness from the calibrated field; the clamp stops an EMA
    // transient from throwing the refraction across the screen.
    let t = clamp(rel, 0.0, 1.25) * optics.slab_depth;

    // The filtered texel differences over the texel are the thickness
    // gradient and Laplacian. field_filter already ran uv.y against
    // world y.
    let texel = 1.0 / vec2f(textureDimensions(field));
    let step = 2.0 * optics.extent * texel;
    let dpf = optics.slab_depth / optics.field_settled;
    let grad = f.gb / (2.0 * step) * dpf;
    let lap = f.a / (step.x * step.x) * dpf;
    let n = normalize(vec3f(-grad, 1.0));

    // Snell into the water, through the thickness, onto the wall.
    let view = vec3f(0.0, 0.0, -1.0);
    let r = refract(view, n, ETA);
    let path = t / max(-r.z, 0.2);
    let path_rel = path / optics.slab_depth;
    let focus = dot(grad, grad) + t * lap;
    let caustic = clamp(1.0 - (1.0 - ETA) * CAUSTIC * focus, CAUSTIC_FLOOR, CAUSTIC_CEIL);
    let through = dazzle(p + r.xy * path, px2w, FROST * path_rel)
        * caustic
        * exp(-ABSORB * path_rel);

    let fres = F0 + (1.0 - F0) * pow(1.0 - n.z, 5.0);
    let rr = reflect(view, n);
    let sky = mix(HORIZON, ZENITH, 0.5 + 0.5 * dot(rr, optics.up));
    let glint = pow(max(dot(n, optics.h), 0.0), SUN_GLOSS) * optics.glint_gain;

    var col = mix(through, sky, fres) + SUN_TINT * glint;
    if (a < 1.0) {
        col = mix(dazzle(p, px2w, 0.0), col, a);
    }
    return vec4f(col, 1.0);
}
