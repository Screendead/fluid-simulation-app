# M3 — The fluid

*Design record, 2026-08-30. Binds the code with `decisions.md`. The method
decision is D5.*

## 1. Goal

Real water in the box. Jack's directive (verbatim in HANDOFF, 2026-08-30):
a full fluid simulation, physically accurate, with calculated pressure,
density, velocity, acceleration and temperature, as close to real water as
possible at the true physical size of the reference device. The water-style
renderer is explicitly left out: that is M4. M3 renders with the M2
sprites and puts field statistics on screen.

## 2. What "1:1 with real water" means here

Real water is ~10^24 molecules in this box. Every real-time method solves
the continuum equations instead. M3 solves incompressible Navier–Stokes
with SPH at particle spacing d, with real SI constants and measured error.
The deviations from reality, each deliberate and bounded:

| Deviation | Bound |
|---|---|
| Continuum at spacing d (1.5–3 mm; stage 0 fixes it) | Sub-d eddies and droplets do not exist |
| Incompressibility to a target, not exactly | Compression error — max(ρ/ρ₀ − 1, 0), so free-surface deficiency does not count — 0.1% average, 1% max |
| z resolves two particle layers at the chosen spacing | Quasi-3D: particles move and pass in z and M4 gets depth, but z eddies do not exist. Three layers needs d ≤ 1.9 mm, which section 5 rules out on this device. |
| CFL clamp on velocity per substep | Violations counted and shown on screen, never silent |
| Air is vacuum; one-phase fluid | No bubbles, no drag from air |
| Surface tension omitted in M3 | Millimetre droplets behave too heavily; revisit at M4 |

Constants, all SI at 20 °C: density 998.2 kg/m³, dynamic viscosity
1.002 mPa·s, gravity from the sensors (D3), heat capacity 4184 J/(kg·K),
thermal conductivity 0.598 W/(m·K), thermal expansion 2.07×10⁻⁴/K.

Discretisation, fixed here because every number below depends on it:
cubic-spline kernel, smoothing length h = 1.2 d, support radius
2h = 2.4 d, particle mass ρ₀·d³, grid cell = one support radius. The
seeder insets half a spacing from each wall, so the depth quantizes to
⌊(7.65 mm − d)/d⌋ layers: two at d = 2.5 mm. The stage-0 d = 3 mm row
was a single layer — a 2D lattice; its cost numbers stand, its physics
does not.

The box: the visible screen at physical size (458 ppi, M2's constant) by
the device's 7.65 mm depth. The simulation is 3D in a thin slab — real
water in a slab moves in 3D, and M4's screen-space renderer needs depth.
2D is rejected in D5.

## 3. Temperature, honestly

Temperature is transported per particle and is physically real, and its
real signal is microkelvin. The arithmetic, so the lens design is a
decision and not an apology:

- Pressure work, built: T·β/(ρ·c_p) · Dp/Dt, with Dp/Dt the substep
  pressure delta from the solve. The coefficient at the constants above
  is 0.0145 K/MPa (0.018 is 25 °C water). The hydrostatic floor of
  ~1.5 kPa upright gives ~22 µK — tens of microkelvin.
- Viscous dissipation, built: µ·(∇v)² / (ρ·c_p) — the ρ is load-bearing;
  without it the term is a thousandfold too hot. µK/s at slosh shear.
- Diffusion, built: SPH Laplacian with real conductivity.
- Thermal expansion feedback on density: β × tens of µK ≈ 10⁻⁸
  relative — negligible by arithmetic, recorded here, not built.

The M5 temperature lens auto-scales its colour range to the live min–max,
so µK structure is visible without faking magnitudes.

## 4. The solver

DFSPH (D5). Per substep — the whole loop lives inside the substeps;
nothing neighbour-dependent survives a position update:

1. Rebuild the neighbour grid: counting sort by cell, fused to four
   dispatches (clear, count, one single-workgroup scan — serial per
   thread over ≤ 32 cells each, enough for every grid this record
   allows — and scatter). Validated on the device (scan against the CPU
   reference: PASS, 9 configurations, 2026-08-30).
2. Density and the DFSPH factor, with analytic planar-wall kernel
   integrals for both: the box is six flat walls, so the truncated
   support integral has a closed form in wall distance — no boundary
   particles. Without this every particle in the slab under-reads
   density, z-walls being nearer than one support everywhere.
3. Divergence-free solve; semi-implicit Euler under the body force and
   Morris viscosity; constant-density solve; position update;
   temperature sources and diffusion (section 3).

The substep count and each dt are CPU decisions at encode time, from
the CFL bound dt ≤ 0.4·d/v_max. v_max crosses from the GPU by a
one-frame-stale asynchronous readback — four bytes, the same
non-blocking slot machinery as the timestamps, never a stall. Staleness
is safe by construction: the GPU-side velocity clamp enforces the dt
the CPU actually encoded, and every clamp is counted and shown.

The build fixed five details the list above leaves open, each a
decision, not an accident:

- Fusion: 13 dispatches a substep — clear, count, scan, scatter,
  density+factor fused, one divergence iteration (kappa then apply),
  forces (body force, Morris viscosity, and both neighbour-sweep
  temperature sources in one pass), two constant-density iterations,
  integrate. Fixed iteration counts: a convergence readback would
  stall the pipeline, and the on-screen compression stat is the live
  adequacy check. Raise the counts before raising substeps.
- Both kappas clamp at zero: the solver pushes, never pulls. Tensile
  suction at the free surface is the known artifact this avoids.
- Pressure in pascals is kappa times rho, the constant-density solve's
  accumulated stiffness. The dimensional chain (alpha in m^5/kg, kappa
  in m^2/s^2) closes exactly; the settled hydrostatic column is the
  empirical oracle.
- The factor's denominator guard is 1e-4 kg^2/m^8 — numerics, not
  physics. It bites only for a particle with no neighbours and no wall
  in range, whose kappa is zero anyway.
- Walls are free-slip in M3: the position clamp zeroes the normal
  velocity component only. A no-slip wall needs a viscous wall term;
  deferred with the wall-overlap table.

The substep count is dynamic — n = ceil(dt·v_max / 0.4 d), clamped to
the FLUID_SIM cap (the knob is now a ceiling, not a count). At rest n
is 1.

An unset FLUID_SIM now defaults to a cap of 7, so an icon launch runs
the solver. Zero selects the M2 demo, by explicit request only. Before
2026-08-31 the default was zero: every icon launch showed the demo.

Per-substep uniforms go through push constants (`immediate_size`).
There is no fallback branch: wgpu 30's Metal backend grants immediates
unconditionally (wgpu-hal metal/adapter.rs), both targets are Metal,
and a branch no run reaches is banned by CLAUDE.md section 7.
`Queue::write_buffer` allocates a staging buffer per call (wgpu 30
source) and does not belong in a substep loop.

Then draw with the M2 sprite pass, colour by speed, plus on-screen
field statistics: compression error %, pressure min–max, temperature
min–max, substep count, clamp count. Every field has a reader from day
one. Reader order follows from that rule: the DFSPH factor joins the
density sweep when the divergence-free solve arrives to read it, not
before.

Three measured facts about the wall integrals (quadrature,
2026-08-30):

- The closed forms are exact for a plane: the piecewise polynomials in
  `sim.rs::wall_density` match brute quadrature to 1e-4 at 40 wall
  distances, and the gradient form to the same bound.
- The additive per-wall sum double-counts where two clip regions
  overlap inside one support ball. The band is the 6 mm perimeter:
  +1–2% density at a side-wall edge, +3.6% at a three-wall corner.
  Opposite z-walls never overlap, so the slab interior is exact.
  Accepted and recorded; the rejected fix is a pairwise-overlap table
  (~0.6% residual, one more lookup per particle per substep).
- The continuum fill overshoots the pristine seeded lattice by ~2.2%
  at the wall-adjacent layer — midpoint-rule undershoot of the steep
  kernel at one spacing, not an algebra error. A flowing fluid
  decorrelates and matches the continuum; the at-rest exit measurement
  reads this bias in the bottom particle row.

### Stage-2 measurement

Reference device, 2026-08-31, d = 2.5 mm (1,620 particles, 1,568
cells), 7 substeps, 43 dispatches a frame (6 per substep + the
reduction): interval p50 = p99 = 8,334 µs — 120 Hz held; gpu p50
6,320 µs, p99 6,650 µs; encode p50 4,200 µs; 66.5 MB. Two findings:

- The chain is verified twice over: a headless GPU test seeds the
  lattice, runs one rebuild + density sweep and asserts the raw
  density band (0.84 rho0 mean, 0.96 max — the half-full slab reads
  under rest at seed, so clamped compression is honestly zero).
- Pressure-less transport on a tilted phone slides every particle
  into one corner point: the stat saturates at ~300 rho0, which is
  1,620 coincident particles times m·W(0), exactly. The metric works;
  the missing solve is the physics.

### Stage-3 measurement

Reference device, 2026-08-31, d = 2.5 mm, cap 7 substeps, ~15
dispatches a substep. After ~26 s of live motion: compression avg
0.017% max 0.63% — inside the exit target while moving; rho 268–1005;
pressure 0–475 Pa; temperature −610..+244 µK; n breathing 3–6;
interval p50 = p99 = 8,334 µs; gpu p50 3.3–7.3 ms. Two defects found
by measurement, both now pinned by tests:

- The fused forces pass read neighbour velocities while writing its
  own — a same-dispatch race, invisible at the uniform seed state. It
  is now an eval/apply split; the one-second settle test guards it.
- Frame zero encodes dt = 0 by design, and kappa divides by dt
  squared: max(0,0)/0 is NaN, and clamp(NaN, lo, hi) parked all 1,620
  particles at the box corner. Every prior "corner dot" deploy was
  this. A zero dt now encodes no solve.

The one-second flat settle on the Mac GPU: compression max 0.13%,
rho max 999.5, pressure max 127 Pa, zero clamps.

### The solver ramp

Reference device, 2026-08-31, full solver, cap 7, FLUID_SPACING knob:

| d | particles | interval p50 / p99 µs | compression, settled |
|---|---|---|---|
| 2.5 mm | 1,620 | 8,334 / 8,334 | 0.017% avg, 0.63% max |
| 2.0 mm | 2,584 | 8,334 / 22,399 | in target; drops frames in slosh |
| 1.75 mm | 5,031 | 8,334 / 47,057 | 47% max in slosh — solver saturates |
| 1.5 mm | 9,200 | 77,128 / 95,993 | broken; thermal "serious" |

The default stays 2.5 mm: the only config that holds the cadence
clean. 2.0 mm is the next step and waits on the optimisation pass —
its CPU encode alone spikes to 22 ms at high substep counts.

### The visual layer — one-pixel tracers

Jack's directive, 2026-08-31: each particle no more than one pixel.
The solver cannot carry a hundredfold more particles, so the visuals
decouple: 262,144 massless tracers advect through the solved velocity
field and draw as single-pixel points, colour by speed. The field is
splatted once a frame to the neighbour grid (16.16 fixed point — f32
storage has no atomic add) and sampled trilinearly, so the whole layer
is four dispatches a frame and never enters a substep. An unsampled
tracer sits still until the fluid returns to it; re-seeding is
deferred. FLUID_TRACERS sets the count; zero restores the solver
sprites. Measured on the device: 262,144 tracers double the GPU frame
(bunched-tracer additive overdraw) and drop hard slosh to 60 Hz;
131,072 holds p50 = p99 = 8,334 µs in motion and is the default.

### The exit measurement

Reference device, 2026-08-31, default spacing, 262,144 tracers.
~105 s settled still (shallow tilt, p ~180 Pa): compression avg
0.11-0.16%, max ~2.0%; v 0.03; n = 1; ~0.4 clamps a second; interval
p50 = p99 = 8,334 µs throughout. ~70 s hard slosh: pressure peaks
2.4 kPa, n breathing 3-7, compression transients to 19% max with
recovery below 1% between shakes; interval degrades to p50 16,668 µs
during the hardest shaking — the tracer overdraw and a saturated
substep cap together exceed the frame.

Against the targets: settled avg 0.13% misses 0.1% narrowly; settled
max ~2% misses 1%, and the max particle is the wall-adjacent layer
whose +2.2% estimator bias section 4 records — the field away from
the walls is inside target. The slosh cadence criterion was met by
the solver alone (pre-tracer captures) and is broken by the visual
layer at 262,144 tracers, not by the physics.

## 5. Resolution and budget — measured, not asserted

CFL arithmetic at v_max = 2 m/s (hard shake): d = 0.33 mm → dt ≤ 66 µs →
~126 substeps per 8.33 ms frame — dead. d = 1.5 mm → dt ≤ 300 µs → ~28.
d = 2 mm → ~21. d = 3 mm → ~14. Counts in the slab (interior ≈ 154 × 71 ×
7.65 mm): d = 1.5 mm → ~25k particles; 2 mm → ~10k; 3 mm → ~3k.

Stage 0 is a microbenchmark on the reference device before the solver
exists: seed N, build the grid, run K density sweeps per frame; ramp N
and K; read the GPU timestamp span.

Measured, reference device, 2026-08-30, Release (grid build + K density
sweeps per frame; scan validation PASS in all runs):

| d | particles | cells | k=1 gpu p50 | k=9 | k=25 |
|---|---|---|---|---|---|
| 3 mm | 550 | 1,152 | 169 µs | 1,296 µs | 3,152 µs |
| 2 mm | 2,584 | 2,380 | 411 µs | 3,403 µs | 6,512 µs |
| 1.5 mm | 9,200 | 4,950 | 1,157 µs | 6,602 µs | 6,532 µs |

Two findings bind the budget protocol. First, dispatch overhead
dominates small dispatches: per-particle sweep cost falls three-fold
from 550 to 9,200 particles, so the solver must prefer fewer, fatter
dispatches. Second, the wall-clock GPU span measures the frequency the
governor chose, not the work: every heavy configuration above holds a
perfect 120 Hz while reading the same ~6.5 ms span — the governor
clocks the GPU to finish just inside the display cadence. The span is
therefore not a work meter below saturation. The M3 budget meter is the
cadence ceiling: raise the per-frame work until interval p99 breaks
8,334 µs; the largest work that holds is the budget.

The ceiling, measured the same day (interval p50/p99 in µs):

| d | particles | k=25 | k=50 | k=100 | k=200 |
|---|---|---|---|---|---|
| 2 mm | 2,584 | 8,334/8,334 | 8,334/8,334 | 8,334/16,668 | 16,668/33,335 |
| 1.5 mm | 9,200 | 8,334/8,334 | 14,149/20,908 | 33,335/43,396 | 66,815/77,687 |

At saturation the span becomes a work meter again, and both spacings
agree on the true cost: ~36 ns per particle per sweep (18.1 ms / 200
sweeps at 2,584; 34.1 ms / 100 at 9,200). The ceiling is then
8.3 ms / (N × 36 ns) dispatches a frame, floored by ~50–90 µs of
overhead per dispatch at small N.

**The choice.** DFSPH spends roughly 14 solver dispatches a substep
before fusion, plus 4 for the per-substep grid rebuild: ~18. Affordable
clamp speed scales as 0.4·d·substeps/8.33 ms, so coarser spacing wins
twice: fewer particles buy more substeps, and d itself widens the CFL
bound. Start at **d = 2.5 mm**: ~1,600 particles seeded (the two-layer
quantisation means this is ~60% of half the box's mass, not half),
~7 substeps a frame, velocity clamp ≈ 0.9 m/s. Honesty about the
clamp: a gravity fall reaches 0.9 m/s in ~41 mm, so any drop beyond a
quarter of the screen clamps — not only hard shakes. The counter shows
every clamp; raising the ceiling raises the clamp, which is what the
d = 2 mm stretch goal and dispatch fusion are for. d = 1.5 mm is out
on this device: two substeps cannot integrate a slosh honestly, and
three z-layers (d ≤ 1.9 mm) costs more sweeps than the ceiling holds.

## 6. Deferrals, explicit

- The water-style renderer: M4, by Jack's directive above.
- The lens dropdown UI: M5 (HANDOFF roadmap; Jack's directive of
  2026-08-30 on the default view binds it).
- Stillness sleep ("idle costs nothing"): M6, as in the M2 record.
- Rotation, Coriolis and Euler forces: M7 (O6). The box frame stays
  non-rotating in M3.

## 7. Tested and exercised

Pure and tested: the CFL arithmetic, kernel normalisation, the prefix
scan (against a CPU scan), grid cell indexing, the temperature source
terms. Exercised on device and simulator, untested: the WGSL solver
passes, kept minimal per pass. The on-screen field statistics are the
standing exercise of every computed field.

## 8. Exit

- [x] Stage-0 microbench table in this record; d and count chosen from
      it (2026-08-30).
- [x] Prefix scan validated on the reference device against a CPU
      reference (PASS, 9 configurations, 2026-08-30).
- [ ] At rest, phone upright: compression error inside target; floor
      pressure reads hydrostatic ~1.5 kPa (upright is the stated
      orientation; flat reads ~75 Pa); temperature drift bounded.
      Numbers in HANDOFF.
- [ ] Under Jack's hand: a convincing slosh, 120 Hz interval p99 within
      budget over a minute, measured and in HANDOFF.
- [ ] Gate and CI green.
