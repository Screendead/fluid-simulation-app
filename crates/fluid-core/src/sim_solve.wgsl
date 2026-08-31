// The DFSPH solver (D5), one file: density and factor, forces and
// temperature sources, the divergence-free and constant-density solves,
// and the integrate step. Kernel and wall integrals mirror sim.rs; a
// divergence is a bug. Walls are analytic (M3 record section 4): a
// static wall joins the factor's squared-sum term and the kappa_i force
// but never the kappa_j terms — the paper's static-boundary rule.

struct SimParams {
    box_min: vec3f,
    cell: f32,
    dims: vec3u,
    count: u32,
    h: f32,
    mass: f32,
    rho0: f32,
}

// One block per substep: the CPU decides dt and the CFL clamp speed.
struct Step {
    force: vec3f,
    dt: f32,
    v_clamp: f32,
}

var<immediate> step: Step;

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read_write> positions: array<vec4f>;
@group(0) @binding(2) var<storage, read_write> velocities: array<vec4f>;
@group(0) @binding(3) var<storage, read> counts: array<u32>;
@group(0) @binding(4) var<storage, read> starts: array<u32>;
@group(0) @binding(5) var<storage, read> sorted: array<u32>;
@group(0) @binding(6) var<storage, read_write> density: array<f32>;
@group(0) @binding(7) var<storage, read_write> alpha: array<f32>;
@group(0) @binding(8) var<storage, read_write> kappa: array<f32>;
@group(0) @binding(9) var<storage, read_write> pressure: array<f32>;
@group(0) @binding(10) var<storage, read_write> prev_pressure: array<f32>;
@group(0) @binding(11) var<storage, read_write> temperature: array<f32>;
@group(0) @binding(12) var<storage, read_write> stats: array<f32, 10>;
@group(0) @binding(13) var<storage, read_write> clamp_count: atomic<u32>;
@group(0) @binding(14) var<storage, read_write> accel: array<vec4f>;
@group(0) @binding(15) var<storage, read_write> xsph: array<vec4f>;

// The XSPH blend strength; the DFSPH paper pairs its solver with this
// filter, and without it a settled deep column never stops ringing.
const XSPH_EPS: f32 = 0.1;

// Near-pressure, the repulsive half of Clavet 2005: a second, sharper
// kernel whose pressure is never negative. It removes the pair-clumping
// instability (particles collapsing into strings) that plain SPH
// pressure cannot see, because the summed kernel is blind to spacing
// inside one support radius. Regularization of the discretization, not
// new physics: real water has no such instability to correct.
const K_NEAR: f32 = 3000.0;

const DYNAMIC_VISCOSITY: f32 = 1.002e-3;
const HEAT_CAPACITY: f32 = 4184.0;
const CONDUCTIVITY: f32 = 0.598;
const EXPANSION: f32 = 2.07e-4;

fn kernel(r: f32, h: f32) -> f32 {
    let q = r / h;
    let sigma = 1.0 / (3.14159265 * h * h * h);
    if q < 1.0 {
        return sigma * (1.0 - 1.5 * q * q * (1.0 - 0.5 * q));
    }
    if q < 2.0 {
        let t = 2.0 - q;
        return sigma * 0.25 * t * t * t;
    }
    return 0.0;
}

fn grad_kernel(x: vec3f, r: f32, h: f32) -> vec3f {
    if r < 1e-9 || r >= 2.0 * h {
        return vec3f(0.0);
    }
    let q = r / h;
    let sigma = 1.0 / (3.14159265 * h * h * h);
    var dw: f32;
    if q < 1.0 {
        dw = sigma / h * (-3.0 * q + 2.25 * q * q);
    } else {
        let t = 2.0 - q;
        dw = sigma / h * (-0.75 * t * t);
    }
    return dw / r * x;
}

// Mirrors sim.rs::wall_density.
fn wall_density(t: f32) -> f32 {
    if t >= 2.0 {
        return 0.0;
    }
    let t3 = t * t * t;
    if t < 1.0 {
        return 0.5 - 0.7 * t + t3 * (1.0 / 3.0 + t * t * (-0.15 + 0.05 * t));
    }
    return 8.0 / 15.0 - 0.8 * t + t3 * (2.0 / 3.0 + t * (-0.5 + t * (0.15 - t / 60.0)));
}

// Mirrors sim.rs::wall_gradient: |integral of grad W| over the clipped
// region, times h. The direction is the wall normal, into the wall.
fn wall_gradient(t: f32) -> f32 {
    if t >= 2.0 {
        return 0.0;
    }
    let t2 = t * t;
    if t < 1.0 {
        return 0.7 + t2 * (-1.0 + t2 * (0.75 - t * 0.3));
    }
    return 0.8 + t2 * (-2.0 + t * (2.0 + t * (-0.75 + t * 0.1)));
}

fn wall_lo() -> vec3f {
    return params.box_min + vec3f(params.cell);
}

// Sum over the six walls of f(per-wall gradient integral, normal): the
// caller folds each wall's vector term. Returns the summed vector, in
// units of the mass-gradient sum (kg / m^4).
fn wall_grad_sum(pos: vec3f) -> vec3f {
    let lo = wall_lo();
    let hi = -lo;
    let inv_h = 1.0 / params.h;
    let scale = params.rho0 * inv_h;
    var g = vec3f(0.0);
    g.x -= scale * wall_gradient((pos.x - lo.x) * inv_h);
    g.x += scale * wall_gradient((hi.x - pos.x) * inv_h);
    g.y -= scale * wall_gradient((pos.y - lo.y) * inv_h);
    g.y += scale * wall_gradient((hi.y - pos.y) * inv_h);
    g.z -= scale * wall_gradient((pos.z - lo.z) * inv_h);
    g.z += scale * wall_gradient((hi.z - pos.z) * inv_h);
    return g;
}

fn cell_base(pos: vec3f) -> vec3u {
    return min(
        vec3u((pos - params.box_min) / params.cell),
        params.dims - vec3u(1u),
    );
}

// Density, wall fill, and the DFSPH factor in one sweep. Static walls
// join the squared-sum term only. The denominator guard is numerics,
// not physics: it only bites for a particle with no neighbours and no
// wall in range, whose kappa is zero anyway.
@compute @workgroup_size(256)
fn density_factor(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let pos = positions[id.x].xyz;
    let base = cell_base(pos);
    var rho = 0.0;
    var rho_near = 0.0;
    var grad_sum = vec3f(0.0);
    var grad_sq = 0.0;
    for (var dz = -1i; dz <= 1i; dz++) {
        for (var dy = -1i; dy <= 1i; dy++) {
            for (var dx = -1i; dx <= 1i; dx++) {
                let coord = vec3i(base) + vec3i(dx, dy, dz);
                if any(coord < vec3i(0)) || any(coord >= vec3i(params.dims)) {
                    continue;
                }
                let c = (u32(coord.z) * params.dims.y + u32(coord.y)) * params.dims.x
                    + u32(coord.x);
                let end = starts[c] + counts[c];
                for (var k = starts[c]; k < end; k++) {
                    let x = pos - positions[sorted[k]].xyz;
                    let r = length(x);
                    rho += params.mass * kernel(r, params.h);
                    let wn = max(1.0 - r / params.h, 0.0);
                    // r > 0 excludes self, whose constant term would
                    // only offset every particle equally.
                    rho_near += select(0.0, wn * wn * wn, r > 0.0);
                    let mg = params.mass * grad_kernel(x, r, params.h);
                    grad_sum += mg;
                    grad_sq += dot(mg, mg);
                }
            }
        }
    }
    let lo = wall_lo();
    let hi = -lo;
    let inv_h = 1.0 / params.h;
    var fill = 0.0;
    fill += wall_density((pos.x - lo.x) * inv_h) + wall_density((hi.x - pos.x) * inv_h);
    fill += wall_density((pos.y - lo.y) * inv_h) + wall_density((hi.y - pos.y) * inv_h);
    fill += wall_density((pos.z - lo.z) * inv_h) + wall_density((hi.z - pos.z) * inv_h);
    rho += params.rho0 * fill * 0.978;
    grad_sum += wall_grad_sum(pos);
    density[id.x] = rho;
    // The w lane is dead outside this window: integrate re-zeroes it
    // after forces_eval consumes it, and each invocation writes only
    // its own .w word while neighbours read .xyz — disjoint 4-byte
    // words, no race.
    positions[id.x].w = rho_near;
    alpha[id.x] = rho / max(dot(grad_sum, grad_sum) + grad_sq, 1e-4);
    pressure[id.x] = 0.0;
}

// Morris viscosity and the two neighbour-sweep temperature sources
// share one pass, written to scratch: applying in the same dispatch
// would race the neighbour reads. 0.01 h^2 in the denominators is the
// standard singularity guard.
@compute @workgroup_size(256)
fn forces_eval(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let pos4 = positions[id.x];
    let pos = pos4.xyz;
    let vel = velocities[id.x].xyz;
    let temp = temperature[id.x];
    let rho_i = density[id.x];
    let base = cell_base(pos);
    var visc = vec3f(0.0);
    var heat = 0.0;
    var blend = vec3f(0.0);
    var near = vec3f(0.0);
    for (var dz = -1i; dz <= 1i; dz++) {
        for (var dy = -1i; dy <= 1i; dy++) {
            for (var dx = -1i; dx <= 1i; dx++) {
                let coord = vec3i(base) + vec3i(dx, dy, dz);
                if any(coord < vec3i(0)) || any(coord >= vec3i(params.dims)) {
                    continue;
                }
                let c = (u32(coord.z) * params.dims.y + u32(coord.y)) * params.dims.x
                    + u32(coord.x);
                let end = starts[c] + counts[c];
                for (var k = starts[c]; k < end; k++) {
                    let j = sorted[k];
                    let x = pos - positions[j].xyz;
                    let r = length(x);
                    let gw = grad_kernel(x, r, params.h);
                    let f = dot(x, gw) / (r * r + 0.01 * params.h * params.h);
                    let pair = params.mass / (rho_i * density[j]) * f;
                    let dv = vel - velocities[j].xyz;
                    let wn = max(1.0 - r / params.h, 0.0);
                    near += (pos4.w + positions[j].w) * wn * wn * (x / max(r, 1e-5));
                    visc += 2.0 * DYNAMIC_VISCOSITY * pair * dv;
                    blend -= params.mass / density[j] * kernel(r, params.h) * dv;
                    // Cleary-Monaghan diffusion, then half the pair's
                    // viscous dissipation; f < 0 carries the signs.
                    heat += 2.0 * CONDUCTIVITY / HEAT_CAPACITY * pair
                        * (temp - temperature[j]);
                    heat -= 0.5 * DYNAMIC_VISCOSITY / HEAT_CAPACITY * pair * dot(dv, dv);
                }
            }
        }
    }
    accel[id.x] = vec4f(visc + K_NEAR * near, heat);
    xsph[id.x] = vec4f(blend, 0.0);
}

@compute @workgroup_size(256)
fn forces_apply(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let a = accel[id.x];
    velocities[id.x] = vec4f(
        velocities[id.x].xyz + step.dt * (step.force + a.xyz) + XSPH_EPS * xsph[id.x].xyz,
        0.0,
    );
    temperature[id.x] += step.dt * a.w;
}

// One divergence-free iteration, kappa half: Drho/Dt from the predicted
// velocities, then kappa^v = Drho/Dt * alpha / dt, clamped to push only.
@compute @workgroup_size(256)
fn div_kappa(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let pos = positions[id.x].xyz;
    let vel = velocities[id.x].xyz;
    let base = cell_base(pos);
    var drho = 0.0;
    for (var dz = -1i; dz <= 1i; dz++) {
        for (var dy = -1i; dy <= 1i; dy++) {
            for (var dx = -1i; dx <= 1i; dx++) {
                let coord = vec3i(base) + vec3i(dx, dy, dz);
                if any(coord < vec3i(0)) || any(coord >= vec3i(params.dims)) {
                    continue;
                }
                let c = (u32(coord.z) * params.dims.y + u32(coord.y)) * params.dims.x
                    + u32(coord.x);
                let end = starts[c] + counts[c];
                for (var k = starts[c]; k < end; k++) {
                    let j = sorted[k];
                    let x = pos - positions[j].xyz;
                    drho += params.mass
                        * dot(vel - velocities[j].xyz, grad_kernel(x, length(x), params.h));
                }
            }
        }
    }
    // A wall is fluid that never moves: approaching it raises density.
    drho += dot(vel, wall_grad_sum(pos));
    kappa[id.x] = max(drho, 0.0) * alpha[id.x] / step.dt;
}

fn apply_kappa(id: u32) {
    let pos = positions[id].xyz;
    let base = cell_base(pos);
    let k_i = kappa[id] / density[id];
    var dv = vec3f(0.0);
    for (var dz = -1i; dz <= 1i; dz++) {
        for (var dy = -1i; dy <= 1i; dy++) {
            for (var dx = -1i; dx <= 1i; dx++) {
                let coord = vec3i(base) + vec3i(dx, dy, dz);
                if any(coord < vec3i(0)) || any(coord >= vec3i(params.dims)) {
                    continue;
                }
                let c = (u32(coord.z) * params.dims.y + u32(coord.y)) * params.dims.x
                    + u32(coord.x);
                let end = starts[c] + counts[c];
                for (var k = starts[c]; k < end; k++) {
                    let j = sorted[k];
                    let x = pos - positions[j].xyz;
                    dv += params.mass * (k_i + kappa[j] / density[j])
                        * grad_kernel(x, length(x), params.h);
                }
            }
        }
    }
    dv += k_i * wall_grad_sum(pos);
    velocities[id] = vec4f(velocities[id].xyz - step.dt * dv, 0.0);
}

@compute @workgroup_size(256)
fn div_apply(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    apply_kappa(id.x);
}

// Warm start for the constant-density solve: last substep's converged
// pressure, half-applied, is the canonical cure for hydrostatic
// ringing — without it the solver re-fights gravity from zero every
// substep and the settled fluid flickers.
@compute @workgroup_size(256)
fn den_warm(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let k = 0.5 * prev_pressure[id.x] / density[id.x];
    kappa[id.x] = k;
    pressure[id.x] = k * density[id.x];
}

// One constant-density iteration, kappa half: predict rho* one dt ahead,
// kappa = (rho* - rho0) * alpha / dt^2, clamped to push only. The
// applied pressure kappa * rho accumulates for the stats and the
// temperature's pressure work.
@compute @workgroup_size(256)
fn den_kappa(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let pos = positions[id.x].xyz;
    let vel = velocities[id.x].xyz;
    let base = cell_base(pos);
    var drho = 0.0;
    for (var dz = -1i; dz <= 1i; dz++) {
        for (var dy = -1i; dy <= 1i; dy++) {
            for (var dx = -1i; dx <= 1i; dx++) {
                let coord = vec3i(base) + vec3i(dx, dy, dz);
                if any(coord < vec3i(0)) || any(coord >= vec3i(params.dims)) {
                    continue;
                }
                let c = (u32(coord.z) * params.dims.y + u32(coord.y)) * params.dims.x
                    + u32(coord.x);
                let end = starts[c] + counts[c];
                for (var k = starts[c]; k < end; k++) {
                    let j = sorted[k];
                    let x = pos - positions[j].xyz;
                    drho += params.mass
                        * dot(vel - velocities[j].xyz, grad_kernel(x, length(x), params.h));
                }
            }
        }
    }
    drho += dot(vel, wall_grad_sum(pos));
    let rho_star = density[id.x] + step.dt * drho;
    let k = max(rho_star - params.rho0, 0.0) * alpha[id.x] / (step.dt * step.dt);
    kappa[id.x] = k;
    pressure[id.x] += k * density[id.x];
}

@compute @workgroup_size(256)
fn den_apply(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    apply_kappa(id.x);
}

// Close the substep: CFL clamp (counted, never silent), position
// update, inelastic walls, and the temperature's pressure work from
// this substep's pressure delta.
@compute @workgroup_size(256)
fn integrate(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    var v = velocities[id.x].xyz;
    let speed = length(v);
    if speed > step.v_clamp {
        v *= step.v_clamp / speed;
        atomicAdd(&clamp_count, 1u);
    }
    var p = positions[id.x].xyz + v * step.dt;
    let lo = wall_lo();
    let hi = -lo;
    let clamped = clamp(p, lo, hi);
    v = select(v, vec3f(0.0), clamped != p);
    positions[id.x] = vec4f(clamped, 0.0);
    velocities[id.x] = vec4f(v, 0.0);
    let dp = pressure[id.x] - prev_pressure[id.x];
    prev_pressure[id.x] = pressure[id.x];
    temperature[id.x] += temperature[id.x] * EXPANSION
        / (density[id.x] * HEAT_CAPACITY) * dp;
}

var<workgroup> red_a: array<f32, 256>;
var<workgroup> red_b: array<f32, 256>;
var<workgroup> red_c: array<f32, 256>;
var<workgroup> red_d: array<f32, 256>;

fn reduce_pair(local: u32) {
    workgroupBarrier();
    for (var off = 128u; off > 0u; off >>= 1u) {
        if local < off {
            red_a[local] += red_a[local + off];
            red_b[local] = max(red_b[local], red_b[local + off]);
            red_c[local] = min(red_c[local], red_c[local + off]);
            red_d[local] = max(red_d[local], red_d[local + off]);
        }
        workgroupBarrier();
    }
}

// The frame's field statistics in one workgroup, three strided passes:
// compression avg/max + rho min/max, pressure min/max + v_max, then
// temperature min/max. stats[9] mirrors the cumulative clamp counter.
@compute @workgroup_size(256)
fn reduce_stats(@builtin(local_invocation_id) local: vec3u) {
    let l = local.x;
    var s = 0.0;
    var m = 0.0;
    var lo = 1e30;
    var hi = -1e30;
    for (var i = l; i < params.count; i += 256u) {
        let c = density[i] / params.rho0 - 1.0;
        s += max(c, 0.0);
        m = max(m, c);
        lo = min(lo, density[i]);
        hi = max(hi, density[i]);
    }
    red_a[l] = s;
    red_b[l] = max(m, 0.0);
    red_c[l] = lo;
    red_d[l] = hi;
    reduce_pair(l);
    if l == 0u {
        stats[0] = red_a[0] / f32(params.count);
        stats[1] = red_b[0];
        stats[2] = red_c[0];
        stats[3] = red_d[0];
    }
    workgroupBarrier();
    s = 0.0;
    lo = 1e30;
    hi = -1e30;
    var vmax = 0.0;
    for (var i = l; i < params.count; i += 256u) {
        lo = min(lo, pressure[i]);
        hi = max(hi, pressure[i]);
        vmax = max(vmax, length(velocities[i].xyz));
    }
    red_a[l] = 0.0;
    red_b[l] = vmax;
    red_c[l] = lo;
    red_d[l] = hi;
    reduce_pair(l);
    if l == 0u {
        stats[4] = red_c[0];
        stats[5] = red_d[0];
        stats[6] = red_b[0];
    }
    workgroupBarrier();
    lo = 1e30;
    hi = -1e30;
    for (var i = l; i < params.count; i += 256u) {
        lo = min(lo, temperature[i]);
        hi = max(hi, temperature[i]);
    }
    red_a[l] = 0.0;
    red_b[l] = 0.0;
    red_c[l] = lo;
    red_d[l] = hi;
    reduce_pair(l);
    if l == 0u {
        stats[7] = red_c[0];
        stats[8] = red_d[0];
        stats[9] = f32(atomicLoad(&clamp_count));
    }
}
