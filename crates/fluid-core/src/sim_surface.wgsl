// The liquid surface: threshold the splatted particle field so the
// liquid/air boundary reads as one clear edge instead of dot haze.

@group(0) @binding(0) var field: texture_2d<f32>;
@group(0) @binding(1) var field_sampler: sampler;

// The settled interior field sits near 3; the band is the edge width.
const EDGE_LO: f32 = 0.8;
const EDGE_HI: f32 = 1.6;
const DEEP: vec3f = vec3f(0.02, 0.10, 0.28);

struct FillVertex {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn fill(@builtin(vertex_index) v: u32) -> FillVertex {
    let xy = vec2f(f32((v << 1u) & 2u), f32(v & 2u));
    var out: FillVertex;
    out.clip = vec4f(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2f(xy.x * 0.5, 1.0 - xy.y * 0.5) * 2.0;
    return out;
}

@fragment
fn surface_frag(in: FillVertex) -> @location(0) vec4f {
    let d = textureSample(field, field_sampler, in.uv).r;
    let a = smoothstep(EDGE_LO, EDGE_HI, d);
    return vec4f(DEEP * a, a);
}
