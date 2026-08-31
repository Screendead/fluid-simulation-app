// The liquid surface: threshold the splatted particle field so the
// liquid/air boundary reads as one clear edge instead of dot haze.

@group(0) @binding(0) var field: texture_2d<f32>;
@group(0) @binding(1) var field_sampler: sampler;

// The settled interior field sits near 3; the band is the edge width.
const EDGE_LO: f32 = 0.8;
const EDGE_HI: f32 = 1.6;
// Field value stands in for depth: a thin sheet reads pale, a settled
// column reads deep.
const SHALLOW: vec3f = vec3f(0.09, 0.34, 0.66);
const DEEP: vec3f = vec3f(0.01, 0.09, 0.27);
const RIM: vec3f = vec3f(0.55, 0.85, 1.0);

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
    // a(1-a) peaks on the threshold; the gradient gate confines the
    // line to true falloffs, or a thin flat-lying layer wears it
    // everywhere. Measured in field texels so render size cancels.
    let texels = f32(textureDimensions(field).x) * fwidth(in.uv.x);
    let rim = a * (1.0 - a) * 4.0 * smoothstep(0.15, 0.5, fwidth(d) / texels);
    let body = mix(SHALLOW, DEEP, smoothstep(EDGE_HI, 5.0, d));
    return vec4f(body * a + RIM * rim * 0.6, a);
}
