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

// Akinci 2013 surface tension: the cohesion spline between fluid
// pairs, adhesion against the walls. The curvature term is left out:
// its normals are noise at this particle count and it doubled the
// measured boil (M3 record, curvature table). Both coefficients
// follow from
// the support radius at run time, so a spacing change cannot silently
// detune them: the cleave integral prices the model's tension at
// sigma = (21/7040) gamma rho^2 c^2 in the continuum, the 2.4 h/d
// lattice realizes 0.8665 of that (independent lattice sum,
// 2026-08-31), and gamma inverts the formula at water's 0.0728 N/m.
// Young-Dupre turns the adhesion-to-tension ratio into a contact
// angle; beta buys 110 degrees, measured water on oleophobic phone
// glass (M3 record dial table; 3.3x would be clean glass). The
// balling that shelved tension in the first sweep was correct
// physics for the zero-adhesion wall it ran against.
const SIGMA_WATER: f32 = 0.0728;
const LATTICE_SIGMA: f32 = 0.8665;
const COS_CONTACT: f32 = -0.342;

fn tension_gamma() -> f32 {
    let c = 2.0 * params.h;
    return SIGMA_WATER * 7040.0 / (21.0 * LATTICE_SIGMA * params.rho0 * params.rho0 * c * c);
}

// 1 / 6.417e-4: the adhesion kernel's cleave integral K = K_hat c^2,
// from the same quadrature as wall_adhesion.
fn adhesion_beta() -> f32 {
    let c = 2.0 * params.h;
    return SIGMA_WATER * (1.0 + COS_CONTACT) * 1558.4
        / (LATTICE_SIGMA * params.rho0 * params.rho0 * c * c);
}

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

// Akinci et al. 2013 cohesion spline over the kernel support.
fn cohesion(r: f32) -> f32 {
    let c = 2.0 * params.h;
    if r >= c || r < 1e-9 {
        return 0.0;
    }
    let k = 32.0 / (3.14159265 * pow(c, 9.0));
    let a = (c - r) * (c - r) * (c - r) * r * r * r;
    if 2.0 * r > c {
        return k * a;
    }
    return k * (2.0 * a - pow(c, 6.0) / 64.0);
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


// The kernel's integral over the quarter-space behind two
// perpendicular walls (wedge), and its t1 partial (wedge_d): the
// region the additive per-axis wall fill counts twice. An edge
// contact particle is otherwise over-filled by 5% of rest density,
// which is the spasm engine the flat-pose films showed hugging the
// box perimeter. Degree-6 fits of the exact integrals, pinned to
// quadrature by the sim.rs tests; errors 0.2% and 1.1% of scale.
fn wedge(t1: f32, t2: f32) -> f32 {
    if t1 * t1 + t2 * t2 >= 4.0 {
        return 0.0;
    }
    return (2.5005807e-01
        + t2 * (-3.4681153e-01
            + t2 * (-4.0986882e-02
                + t2 * (3.0301620e-01
                    + t2 * (-1.9202736e-01 + t2 * (4.6947582e-02 + t2 * (-3.8543515e-03)))))))
        + t1 * ((-3.4681153e-01
            + t2 * (4.8587248e-01
                + t2 * (1.0092180e-02
                    + t2 * (-2.9978363e-01 + t2 * (1.5320335e-01 + t2 * (-2.2544282e-02))))))
            + t1 * ((-4.0986882e-02
                + t2 * (1.0092180e-02
                    + t2 * (-5.4145610e-02 + t2 * (7.9597369e-02 + t2 * (-2.3350373e-02)))))
                + t1 * ((3.0301620e-01
                    + t2 * (-2.9978363e-01 + t2 * (7.9597369e-02 + t2 * (-8.2431946e-03))))
                    + t1 * ((-1.9202736e-01 + t2 * (1.5320335e-01 + t2 * (-2.3350373e-02)))
                        + t1 * ((4.6947582e-02 + t2 * (-2.2544282e-02))
                            + t1 * (-3.8543515e-03))))));
}

fn wedge_d(t1: f32, t2: f32) -> f32 {
    if t1 * t1 + t2 * t2 >= 4.0 {
        return 0.0;
    }
    return (-3.4611994e-01
        + t2 * (4.4757673e-01
            + t2 * (1.1932939e-01
                + t2 * (-4.6460101e-01
                    + t2 * (2.7637253e-01 + t2 * (-6.2846025e-02 + t2 * (4.3871652e-03)))))))
        + t1 * ((-6.3622823e-02
            + t2 * (2.0521006e-01
                + t2 * (-3.7311339e-01
                    + t2 * (3.7508155e-01 + t2 * (-1.6629713e-01 + t2 * (2.5030129e-02))))))
            + t1 * ((7.8352497e-01
                + t2 * (-1.2658344e+00
                    + t2 * (5.8898945e-01 + t2 * (-1.0772903e-01 + t2 * (1.3535777e-02)))))
                + t1 * ((-4.9470084e-01
                    + t2 * (9.1302071e-01 + t2 * (-3.4330355e-01 + t2 * (2.4034609e-02))))
                    + t1 * ((-2.5605481e-02 + t2 * (-2.0067981e-01 + t2 * (5.9854468e-02)))
                        + t1 * ((8.8898623e-02 + t2 * (3.9914838e-03))
                            + t1 * (-1.7788321e-02))))));
}

fn wedge_dsum(t: f32, p: vec4f) -> f32 {
    return wedge_d(t, p.x) + wedge_d(t, p.y) + wedge_d(t, p.z) + wedge_d(t, p.w);
}

fn wedge_sum(tl: vec3f, th: vec3f) -> f32 {
    var w = wedge(tl.x, tl.y) + wedge(tl.x, th.y) + wedge(th.x, tl.y) + wedge(th.x, th.y);
    w += wedge(tl.x, tl.z) + wedge(tl.x, th.z) + wedge(th.x, tl.z) + wedge(th.x, th.z);
    w += wedge(tl.y, tl.z) + wedge(tl.y, th.z) + wedge(th.y, tl.z) + wedge(th.y, th.z);
    return w;
}

// Sum over the six walls of f(per-wall gradient integral, normal),
// each wall's term relieved of its wedge double-counts against the
// four perpendicular walls. Returns the summed vector, in units of
// the mass-gradient sum (kg / m^4).
fn wall_grad_sum(pos: vec3f) -> vec3f {
    let lo = wall_lo();
    let hi = -lo;
    let inv_h = 1.0 / params.h;
    let tl = (pos - lo) * inv_h;
    let th = (hi - pos) * inv_h;
    let scale = params.rho0 * inv_h;
    let pyz = vec4f(tl.y, th.y, tl.z, th.z);
    let pxz = vec4f(tl.x, th.x, tl.z, th.z);
    let pxy = vec4f(tl.x, th.x, tl.y, th.y);
    var g = vec3f(0.0);
    g.x -= scale * (wall_gradient(tl.x) + wedge_dsum(tl.x, pyz));
    g.x += scale * (wall_gradient(th.x) + wedge_dsum(th.x, pyz));
    g.y -= scale * (wall_gradient(tl.y) + wedge_dsum(tl.y, pxz));
    g.y += scale * (wall_gradient(th.y) + wedge_dsum(th.y, pxz));
    g.z -= scale * (wall_gradient(tl.z) + wedge_dsum(tl.z, pxy));
    g.z += scale * (wall_gradient(th.z) + wedge_dsum(th.z, pxy));
    return g;
}

// The integral of the Akinci adhesion kernel over a wall half-space,
// as a polynomial in u = 2 - d/h pinned to zero value and slope at
// the support edge. The render test pins it to the direct quadrature;
// fit error 0.3%.
fn wall_adhesion(t: f32) -> f32 {
    let u = clamp(2.0 - t, 0.0, 2.0);
    return u * u
        * (1.7847015e-3
            + u * (3.3392196e-3
                + u * (-4.3394677e-3 + u * (1.6711868e-3 + u * (-2.1788750e-4)))));
}

// Acceleration toward each wall inside the support, per unit of
// ADHESION * rho0.
fn wall_adh_sum(pos: vec3f) -> vec3f {
    let lo = wall_lo();
    let hi = -lo;
    let inv_h = 1.0 / params.h;
    var a = vec3f(0.0);
    a.x -= wall_adhesion((pos.x - lo.x) * inv_h);
    a.x += wall_adhesion((hi.x - pos.x) * inv_h);
    a.y -= wall_adhesion((pos.y - lo.y) * inv_h);
    a.y += wall_adhesion((hi.y - pos.y) * inv_h);
    a.z -= wall_adhesion((pos.z - lo.z) * inv_h);
    a.z += wall_adhesion((hi.z - pos.z) * inv_h);
    return a;
}

fn cell_base(pos: vec3f) -> vec3u {
    return min(
        vec3u((pos - params.box_min) / params.cell),
        params.dims - vec3u(1u),
    );
}

// Density, wall fill, the DFSPH factor, and the divergence predictor
// in one sweep: the divergence solve's neighbour traversal is this
// traversal, so its Drho/Dt rides along and the div_kappa dispatch
// disappears. Safe because nothing writes positions.xyz or velocities
// between the old two dispatches. Static walls join the squared-sum
// term only. The denominator guard is numerics, not physics: it only
// bites for a particle with no neighbours and no wall in range, whose
// kappa is zero anyway.
@compute @workgroup_size(256)
fn density_div(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.count {
        return;
    }
    let pos = positions[id.x].xyz;
    let vel = velocities[id.x].xyz;
    let base = cell_base(pos);
    var rho = 0.0;
    var rho_near = 0.0;
    var grad_sum = vec3f(0.0);
    var grad_sq = 0.0;
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
                    let r = length(x);
                    rho += params.mass * kernel(r, params.h);
                    let wn = max(1.0 - r / params.h, 0.0);
                    // r > 0 excludes self, whose constant term would
                    // only offset every particle equally.
                    rho_near += select(0.0, wn * wn * wn, r > 0.0);
                    let gw = grad_kernel(x, r, params.h);
                    // mass stays outside the dot below: distributing it
                    // inside changes the rounding, and this kernel's
                    // claim is bit-equality with the two it replaces.
                    let mg = params.mass * gw;
                    grad_sum += mg;
                    grad_sq += dot(mg, mg);
                    drho += params.mass * dot(vel - velocities[j].xyz, gw);
                }
            }
        }
    }
    let lo = wall_lo();
    let hi = -lo;
    let inv_h = 1.0 / params.h;
    let tl = (pos - lo) * inv_h;
    let th = (hi - pos) * inv_h;
    var fill = 0.0;
    fill += wall_density(tl.x) + wall_density(th.x);
    fill += wall_density(tl.y) + wall_density(th.y);
    fill += wall_density(tl.z) + wall_density(th.z);
    fill -= wedge_sum(tl, th);
    rho += params.rho0 * fill;
    let wall_grad = wall_grad_sum(pos);
    grad_sum += wall_grad;
    density[id.x] = rho;
    // The w lane is dead outside this window: integrate re-zeroes it
    // after forces_eval consumes it, and each invocation writes only
    // its own .w word while neighbours read .xyz — disjoint 4-byte
    // words, no race.
    positions[id.x].w = rho_near;
    let a = rho / max(dot(grad_sum, grad_sum) + grad_sq, 1e-4);
    alpha[id.x] = a;
    pressure[id.x] = 0.0;
    // A wall is fluid that never moves: approaching it raises density.
    drho += dot(vel, wall_grad);
    // The dt guard covers the substep-free test path only; every real
    // substep is orders of magnitude above it, bit-unchanged.
    kappa[id.x] = max(drho, 0.0) * a / max(step.dt, 1e-9);
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
    var tension = vec3f(0.0);
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
                    let corr = 2.0 * params.rho0 / (rho_i + density[j]);
                    tension -= corr * params.mass * cohesion(r) * x / max(r, 1e-9);
                }
            }
        }
    }
    let adh = adhesion_beta() * params.rho0 * wall_adh_sum(pos);
    accel[id.x] = vec4f(visc + K_NEAR * near + tension_gamma() * tension + adh, heat);
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
    // The constant-density warm start rides along: every access in both
    // jobs is at the invocation's own index. Last substep's converged
    // pressure, half-applied, is the canonical cure for hydrostatic
    // ringing — without it the solver re-fights gravity from zero
    // every substep and the settled fluid flickers.
    let k = 0.5 * prev_pressure[id.x] / density[id.x];
    kappa[id.x] = k;
    pressure[id.x] = k * density[id.x];
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
    // Detached spray may outrun CFL: below half rest density there is
    // no neighbourhood to tunnel through, so flight is ballistic and
    // the clamp would only steal throw height (the upright-shake
    // heaviness Jack reported). The interior keeps the full clamp.
    let ceiling = select(
        step.v_clamp,
        3.0 * step.v_clamp,
        density[id.x] < 0.5 * params.rho0,
    );
    if speed > ceiling {
        v *= ceiling / speed;
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
