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

### Tracer recycling

A passive tracer advected in the splatted, cell-averaged field
collapses onto attractors: the sampled field is compressible where the
flow is not. Jack's screen recording of 2026-08-31 showed 131,072
tracers drawn as a few hundred lit pixels after eleven minutes, a thin
thread on the bottom wall, while the solver sat at rest density.
Strays also freeze in cells the fluid has left, so each hard shake
parks more dust on the walls until the fluid sweeps those cells again.

The cure is turnover. Each frame a tracer recycles with probability
dt/TAU, TAU 3 s, and respawns at a random solver particle plus a
quarter-cell jitter, with its speed tag copied from that particle. The
cloud relaxes back to the true fluid distribution with time constant
TAU, whatever the run length. The regression test pools the fluid
against the -x wall, forces a full recycle with one dt-above-TAU
advect pass, and counts tracers right of the pool: zero with the
branch live, 23 stranded without it.

### Five density iterations

Jack reports jitter upright and worse at 45 degrees (2026-08-31). The
corner test reproduces it. At three density iterations, over 15 s in
the corner: rho max 1013.7..1017.6, compr max 1.5..1.9 percent, v
floor 0.089..0.112, clamps 74..93. Five iterations collapse it:
corner rho max 1003.7..1008.1, compr max 0.5..1.0 percent, v floor
0.034..0.045; upright v floor 0.030 with zero clamps. Rho max fell,
so the solver converges onto a truer target: the ringing was
under-convergence, not only the recorded wall bias. Cost: four more
dispatches per substep, about a fifth more solve time; at rest n
stays 1..2 and the budget holds. Two options rejected. Iterations
conditional on n (five calm, three violent) saves time in the shake
regime the optimisation pass owns anyway, and lets the damping
character change with n. The pair-overlap wall correction stays
queued: the corner-versus-upright delta collapsed without it.
(Mac numbers, 2026-08-31; the device capture follows the next
deploy.)

Stray escalation (2026-08-31): Jack reports straggler dots at the
fluid edge; the frame analysis of his second recording shows a dust
field fading out 50..500 capture pixels from the mass, fully replaced
each sample. A stranded tracer waited up to TAU for the field to
return. An unsampled tracer now recycles with time constant
TAU_STRAY, 0.25 s, so dust clears about twelve times faster. The
respawn path is shared with the base recycle the regression test
forces.

Amendment to Five density iterations, same day: later baseline runs
put the upright v floor anywhere in 0.028..0.119 across identical
configs, so the v-floor part of that measurement was partly a lucky
draw. The robust gains are rho max, compression and clamps. A planned
divergence-x2 comparison never ran: its edit failed and the two runs
were baselines. The rest floor is bounded near 0.03 m/s by estimator
noise at 2.5 mm spacing; the next real solver lever is finer spacing,
queued behind the encode optimisation pass.

### The liquid surface

Jack asks for a clear boundary between liquid and air (2026-08-31).
At 18 native pixels per millimetre the 0.03 m/s churn floor is about
500 pixels per second of raw dot motion, so no solver knob makes bare
dots read as still water. The cure is to draw the liquid as a body.
Solver particles splat their kernel footprint into a half-resolution
R16Float field; a fullscreen pass thresholds it with a smoothstep and
lays the body colour under the dots, so the surface is one clear edge
and interior churn stops reading as edge noise. The threshold pair
starts at 0.8..1.6 against an interior field near 3; Jack's eye tunes
it. Budget: the settled frame holds 2.1 ms headroom (gpu p50 6.2 of
8.33 ms, 2026-08-31); the body pass draws the particle count as
half-resolution quads.


### The film harness

Jack rejects the debug loop of phone screen recordings (2026-08-31):
"there has to be a better way to develop & debug this". There is.
`scripts/film.sh` runs the real solver and render passes headless on
the Mac against a scripted force trajectory and writes an mp4:
upright, tilt to the corner, flat on the desk, back, with an optional
two-tone hand tremor (TREMOR=0 films a tripod; SPACING overrides the
particle spacing). The box stays the reference phone's whatever the
render size, because every vertex shader projects from the box
extent, not the resolution. Frames advance at a fixed 1/120 s, so
film is the look oracle only; the phone's stats line stays the cost
oracle. The harness lives behind the `film` feature, off in every
device build, and the gate lints it via --all-features.

### The surface fix and the rim

The first surface build put the fill's uv in 1..2 instead of 0..1, so
the sampler's edge clamp smeared the field's bottom row up the whole
screen: the vertical streaks and the whole-screen wash in Jack's
third recording. Film reproduced it on the desk in one run. Fixed,
the boundary needed weight: the fill now lays a rim line where the
smoothstep crosses its threshold, gated by the field's spatial
gradient measured in field texels so a thin flat-lying layer does not
wear rings, and so the gate survives any render resolution. The field
dropped from half to quarter resolution: the body pass was the whole
regression from 6.2 to 9.2 ms settled gpu p50, and overdraw scales
with field area. Deployed and measured on the device, 2026-08-31:
settled gpu p50 6327..6393 us over a 160 s soak, interval p50 and
p99 both 8334 us, so the whole surface rendering costs about 0.15 ms
against the pre-surface baseline and 120 Hz holds locked.

### The boil, measured, and the missing physics

Film with TREMOR=0 answers what no phone recording could: after five
still seconds the surface still rearranges within 0.17 s. The jitter
Jack reports is real solver churn, not rendering, not his hand, and
the earlier estimator-noise reading of the rest floor was wrong — the
motion is visible surface waves, millimetres tall. The boil meter (a
scratchpad script; median temporal deviation of the per-column
surface height over the settled hold) scores the levers:

| Run | Boil, median mm |
|---|---|
| Baseline | 2.99 |
| XSPH 0.25 | 2.63 |
| Spacing 2.0 mm | 2.07 |
| Wall fill x0.978 | 2.29 |
| Divergence x3 | 2.43 |
| Wall x0.978 + XSPH 0.25 | 2.34 |

Every lever trims; none cures; the two best do not compound. The
signature fits waves the sim cannot damp, not a pump the levers can
starve: this box is 71 x 154 x 9.5 mm and the capillary length of
water is 2.7 mm, so real water at this scale kills millimetre chop
with surface tension, which the sim does not model. Surface tension
is therefore an M3 accuracy gap, not polish, and the next solver
work. Two changes shipped from the sweep: the wall fill scaled by
0.978, recorded above as a calibration of the known +2.2% contact
bias and worth -23% boil alone, and nothing else — XSPH stays 0.1
and one divergence iteration stands, since neither pays for its cost
or its extra damping once the wall is calibrated. h at 1.5x spacing
was tried and diverged at rest; the wall integrals are calibrated
for 1.2 and stay there.

### Surface tension: tried, measured, not shipped

The boil pointed at missing capillary physics, so Akinci cohesion plus
curvature went in behind a normals pass (one extra neighbour sweep, a
sixteenth storage binding, walls entering normals through their
analytic integrals). The film boil meter swept the coefficient:

| gamma | Boil, median mm |
|---|---|
| 0.5 | 1.32 |
| 1.0 | 0.93 |
| 1.5 | 1.14 |
| 2.0 | 1.17 |
| 3.0 | 1.12 |
| 10 | 5.37, seven substeps at rest |
| curvature only, 1.0 | 4.28 |

Best case 0.93 mm, a third of baseline — and unshippable. At every
effective strength the flat pose balls the puddle into one blob mid
screen, where real water at 4.75 mm depth over 71 mm sheets across
the glass (Bond number far above one). The tension able to kill
millimetre chop at 2.5 mm spacing is orders above physical, and at
that strength cohesion beats gravity and wetting. Curvature alone
destabilises: cohesion's near branch is what regularises spacing.
Two open observations, not diagnosed: a standing central fountain at
rest and gummy-looking slosh, both at gamma 1.0. The diff is
preserved outside the repository. The next experiment is wall
adhesion, Akinci's companion term, built on the same analytic wall
integrals: it is the mechanism that makes water wet glass, it fights
the balling directly, and it carries a physical prediction (the
contact angle follows the adhesion-to-cohesion ratio), so it is a new
mechanism rather than a knob. Richer neighbourhoods at 2.0 mm may
also reopen the plain model; that waits on the encode pass.

### The field averages 37 milliseconds — at rest only

Shipped instead, in the rendering domain the water shader deferral
already owns: the splatted field keeps 0.8 of itself each frame (a
decay draw with a constant blend factor) and the body splat scales by
the complement, so the field is an exponential average with a 37 ms
time constant and an unchanged steady state — the thresholds hold.
The rim cannot flicker at frame rate; the boil meter, which samples
at 30 fps and so barely sees frame-rate sparkle, still drops 2.34 to
2.07 mm. The solver's slow hump migration remains and remains the
solver's problem: this smooths the drawn boundary, not the fluid.

Amended 2026-08-31, after Jack's fourth recording. A fixed 37 ms
average is motion blur: at shake speed the surface moves centimetres
inside the time constant, and the boundary smeared into fog across
half the screen. Keep is now a function of v_max — 0.8 below
0.05 m/s, smoothstepped to zero by 0.25 m/s, chased a quarter step
per frame so it cannot pop. The splat scale moved from the shader
into the blend constant (1 - keep), so decay and splat stay exact
complements at every keep and the steady state and thresholds hold
unchanged. At rest nothing changed; in motion the field is raw.

### Near-pressure: the jitter was the pair instability (2026-08-31)

Jack's verdict after the look pass: still jittering like hell upright
and at 45 degrees, and nothing in the project's history moved the
needle. He is right, twice over. First, the metric was soft: the boil
meter has measured the rendered field since the EMA shipped, so its
numbers carried cosmetic smoothing. The film now takes a KEEP pin
(KEEP=0 films the raw field), and the raw baseline is 2.56 mm on the
19 s tripod hold, 2.24 mm on the 6 s sweep hold — with tripod and
handheld raw within 0.04 mm of each other, so tremor contributes
nothing and the churn is all solver.

Second, the churn's root is the pair-clumping (tensile) instability:
summed-kernel SPH pressure is blind to spacing inside one support
radius, so particles collapse into strings and keep rearranging
forever — the stringy filaments visible in every recording were this.
The fix is the repulsive half of Clavet 2005's double-density
relaxation: a second, sharper kernel (1 - r/h)^3 whose pressure is
never negative, applied pairwise. The attractive half is deliberately
left out — it is Clavet's surface tension, and it would rediscover
the Akinci balling failure. Regularization, not new physics: real
water has no such instability to correct.

Near-density rides in positions.w, which integrate re-zeroes every
substep and nothing else reads: zero bytes added. The sweep, 6 s
tripod films, raw field, level = mean wetted height (the volume-
inflation guard), compr = max compression:

| K_NEAR | boil median mm | p90 | level mm | compr % |
|---|---|---|---|---|
| 0 | 2.24 | 2.90 | 47.12 | 0.338 |
| 300 | 2.31 | 3.09 | 46.84 | 0.255 |
| 1000 | 2.59 | 3.36 | 47.23 | 0.266 |
| 3000 | 0.80 | 0.96 | 47.02 | 0.226 |
| 6000 | 0.91 | 1.24 | 47.16 | 0.196 |
| 10000 | 1.72 | 2.03 | 48.67 | 0.195 |

The jump between 1000 and 3000 is a phase change: below it the
repulsion only stirs, above it the clumping attractor breaks and the
packing settles. 10000 starts to inflate the pool. Shipped: 3000 —
boil 2.24 to 0.80 mm raw, level flat, compression better than
baseline. Known debt: the near force does work that the temperature
ledger does not book; at rest speeds the magnitude is microscopic,
and the dT stat range on device is the watch. One visible side
effect: the flat-pose monolayer now settles into a faint hexagonal
texture — the regularized packing showing through the field. Subtle,
and far better than the per-particle leopard it replaced; left as is.

### The refine cut at high n (2026-08-31)

Jack's shake recording also showed pacing collapse: interval p99
33.8 ms, gpu p99 28.9 ms at n 16 (device, 2026-08-31) — the GPU
itself runs 3.5x over the 8.3 ms budget at the violent peak, and the
cpu p99 in the 30s is back-pressure from the swapchain, not encode.
Density error scales with dt squared, so past eight substeps the
warm-started density solve now runs two refine passes instead of
five. The film guard: max compression over the scripted shake is
0.377 % at five refines, 0.381 % at two — identical. The film now
prints that number every run (stderr), so every future solver
experiment carries the guard for free. The cut removes about a
quarter of the dispatches per high-n substep; the violent peak still
exceeds the budget, so the hardest transients will drop frames. The
trade between the 16-substep ceiling and clean pacing is Jack's
call, priced by his next on-device shake.

### The sensor was pumping the pool (2026-08-31)

The substep-floor build behaved opposite to film on device: flat
mostly fixed, reclined unchanged — and the stats line showed why.
The device fluid never settles: v_max parks at 0.13–0.24 m/s
(film tripod: 0.03–0.05), the CFL holds n at 3–4, the clamp fires
~17,000 times a second at rest, and the GPU sits over budget at 60 Hz
under a serious thermal state. The film feeds a constant force; the
phone feeds accelerometer noise. With NOISE=0.15 (m/s^2 RMS, the
new film knob) the desk reproduces every device number: v_max 0.15–
0.24, a clamp storm, reclined jumps back. Jack asked on day one
whether smoothing the raw sensor data would help; the tripod control
only proved solver churn existed without input — it never tested
noise on top. He was right.

Shipped: an adaptive low-pass on the body force in the core, shared
by production and film. Blend factor = deviation / 2 m/s^2, clamped
to 0.1..1: stationary noise meets an ~80 ms time constant, a real
tilt or shake opens the filter within a frame. With noise on, film:
reclined jumps 39 to 1 (v_max settles to 0.012, clamps 3494 to 135),
upright raw boil 0.51 mm — the best measured, from 2.56 this
morning — and the shake film is unchanged in violence, with the
mid-tilt pour showing no visible lag. The settled v_max also lets
the CFL fall back to the floor of two, which unwinds the clamp storm,
the GPU load, and with it the 60 Hz thermal spiral.

### The resting jitter was a convergence failure at one substep (2026-08-31)

Jack, on the near-pressure build: flat pose spasms cascade through
the pool, and a reclined pose (on its back, ~30 % lifted) jumps a
couple of times per second. Both reproduce on film once the harness
gained RECLINE and FLAT poses and a jump meter (per-frame pool-level
delta and per-pixel stir; stills cannot show a 2 Hz event, which is
how the flat check missed it). Attribution: with near-pressure OFF
the jumps are five to ten times worse — the term reduced them; weak
in-plane gravity had simply been hiding the solver's resting noise
in every pose Jack had watched before.

The discriminating test: forcing four substeps at rest kills the
reclined jumping completely (level delta 0.26 mm/frame at one
substep, 0.005 at four, same model). So the noise is not the model:
at a full 8.3 ms substep the density solve cannot converge against
the wall and near-pressure springs, and the residual limit-cycles.
More iterations at one substep help upright (0.80 to 0.45 mm boil at
16 iterations) but sharpen reclined stick-slip (occasional 3.5 mm
avalanches) — tight convergence at a too-long timestep stiffens the
contact instead of stabilizing it.

Shipped: a substep floor of two, and a low-n iteration schedule —
ten density refines at n <= 2, five in between, two past eight.
Film: reclined jumps 0.26 to 0.007 mm/frame (dead calm), upright
tripod raw boil 2.56 (this morning) to 0.73 mm. The rest cost rises
by roughly one substep (~0.6 ms GPU, to be measured on device); the
idle gate reclaims all of it when a settled phone stops stepping.
Flat-pose spasms remain, improved but real: at four substeps flat
only halves, so it is a genuine model error — every particle sits in
the z-wall contact layer under normal load there, where the analytic
wall fill is least accurate. That is the wall-model follow-up's
first target.

### The spray gate: CFL is a validity condition, not physics (2026-08-31)

Jack asked why the sim clamps velocity at all — a correct integrator
should not inject energy. He is right in continuous time. The
injection is discrete: a particle that outruns the spatial
discretization in one substep tunnels into a neighbour's kernel and
takes a huge pressure kick out, which compounds — CFL bounds the step
to the discretization's validity envelope, and the clamp only exists
because the substep count caps at 16 for GPU budget. The accuracy it
costs is exactly the upright-shake heaviness he reported: the ceiling
was 1.92 m/s against real shake throws of 3–5, so throw height capped
near one screen while gravity was never truncated. Flat pose has no
in-plane gravity to compare against, which is why it felt right.

The fix follows the physics of the failure: a particle below half
rest density is detached spray with no neighbourhood to tunnel
through, so its flight is ballistic and safe at any speed. integrate
now gives those particles three times the CFL ceiling; the interior
keeps the full clamp. Film A/B at the same shake instant: without the
gate the ejecta hugs the wall and the ceiling stays dry; with it,
sheets plaster the ceiling and the air fills with spray — at
identical max compression (0.238 vs 0.237 %). Zero memory, two
shader lines, and it sidesteps the ceiling-versus-pacing trade: n is
untouched.

### The look pass (2026-08-31)

Jack's verdict on the first cap-16 recording: "this looks like shit."
He is right, and three of the causes were rendering debts:

- The motion fog above.
- The body-force tint. M2 chose it when sprites were the whole scene;
  under the water it reads as a bruise. The backdrop is now a fixed
  near-black with a cold cast (M2 record amended). The tint's two unit
  tests went with it.
- Flat fill and constant-brightness dots. The fill now ramps pale
  (thin sheet) to deep blue over field value 1.6..5, so the flat pose
  reads as a sheet of water instead of a translucent wash, and the rim
  brightened. Dot brightness now rides on tracer speed — a resting dot
  vanishes instead of speckling the body, and fast water glints. The
  speed colouring Sebastian Lague's fluid video demonstrates was
  already half-built here; the missing half was the fade.

### The splash was clamped away

Jack reports jelly-like motion and no splash under a hard upright
shake (2026-08-31). The stats line held the cause all along: the CFL
velocity clamp is 0.4 x spacing x n / dt, and at the substep ceiling
of seven that is 0.84 m/s — a hard shake throws water at two to five,
so the solver forbade splashing (cumulative clamps in the millions),
and squeezing every speed toward one ceiling reads as jelly. The film
shake trajectory (SHAKE=1) proves it: at cap seven the fluid stays
one coherent tongue; at sixteen it sheets up the walls and across the
ceiling in droplet streaks. The default substep ceiling rises to
sixteen (FLUID_SIM still overrides). Rest cost is unchanged — the CFL
picks one substep at rest — and the violent-moment cost lands only
while shaking; the device shake capture prices it. The remaining
jelly suspect is the tracer grid's cell-averaged field, which cannot
show a vortex smaller than two cells; a finer tracer grid is the
queued experiment.

Thermal, same day: a long screen-on soak reads "serious" on the
device. The continuous load is the unbuilt idle gate ("Idle costs
nothing"), now the next feature ahead of everything but this splash
fix.

## The idle gate

CLAUDE.md rule: idle costs nothing. A still phone ran the full solver at
120 Hz, over the frame budget once thermal throttle set in (rest gpu p50
8.5 ms hot against a 6.3 ms cool baseline, device, 2026-08-31). The heat
was self-inflicted: the resting sim heated the phone, the throttle slowed
the GPU, the slow GPU missed frames.

`IdleGate` (render.rs) sleeps the sim when the pool and the phone are both
still, and wakes it on the first sign of motion.

- Sleep: v_max under 0.12 m/s and force deviation under 0.5 m/s^2 for 180
  consecutive frames. Device rest v_max wanders 0.03..0.12 under sensor
  noise; hand tremor holds v_max above the threshold, so a held phone
  never sleeps (film: TREMOR=1 gives idle 0).
- Wake, three tests against the live filter: deviation over 1.2 m/s^2 (a
  shake, one tick); the smoothed force more than 1.5 degrees from its
  sleep snapshot (a slow tilt); magnitude shifted over 0.3 m/s^2 (a lift).
- Asleep, `frame()` returns false before any encode: no GPU work, no
  present, no interval sample. The filter and the gate run every tick —
  frozen inputs cannot wake anything. The shell drops the display link to
  30 Hz on false and restores 120 Hz on true, so wake latency is at most
  two visually still frames.

Film oracles (2026-08-31, NOISE=0.15):

| Film | Result |
|---|---|
| FLAT soak, 30 s | sleep at frame 506, idle 3094, zero false wakes |
| WAKE (6 s flat, then eased tilt to recline) | sleep 503, wake 768 — 0.4 s into the tilt, at 1.5 degrees accumulated, water visually unmoved |
| TREMOR=1 trajectory | idle 0: a held phone never sleeps |

Metrology caveat: boil and jump films measure a settled window, and the
gate freezes exactly that window. Every metrology film must set IDLE=0.

Device measurement pending: rest power and thermal recovery with the gate
in, cool phone, dated. The 30 Hz nap tick costs one filter apply and one
gate check on the CPU; the GPU encodes nothing.

## The cost pass, round one

A six-direction research panel (dispatch fusion, grid reuse, iteration
economics, layout and precision, tracers, encode and render passes) priced
the active frame on 2026-08-31. Its corrected anatomy: 68 dispatches per
frame at n=2, 260 at n=16 — and at rest the solver is ~1.1 ms of the
6.3 ms GPU frame; the tracer layer and render passes own the rest. The
implemented and rejected items land in this section as they settle.

One finding changes the verification story for everything after it: the
sim is not run-to-run deterministic. The scatter allocates within-cell
slots with atomicAdd, so neighbour order is scheduling-dependent and every
float accumulation reorders run to run. Measured (Mac, 2026-08-31, same
binary, FLAT NOISE=0.15 IDLE=0, 480 frames): compr max 0.037..0.088%,
clamps 126..199, v_max end 0.030..0.055, film hashes all distinct. A
"bit-identical" change is therefore proved by algebra and tests, never by
comparing film output; a changed film guard number inside that band means
nothing.

Implemented: advect reads the tracer velocity grid through a plain
read-only view of the same buffer (binding 5, array of vec4i) instead of
4.19 million device-scope atomic loads per frame; splat keeps the atomic
view — concurrent adds are the only reason the buffer is atomic. Panel
prediction: 650..2,950 us per frame at every n, since advect runs once
per frame. Algebraically identical maths; 24/24 tests pass. Device
measurement pending.

### Round one, implemented

Four changes landed 2026-08-31, all pushed, each gate-green. None has a
device number yet; the runbook below prices them all in one session.

| Change | Commit | Panel prediction (A15, cool) |
|---|---|---|
| advect reads the velocity grid without atomics | "Read the tracer velocity grid without atomics in advect" | 650..2,950 us/frame, every n |
| den_warm folded into forces_apply | "Fold the density warm start into the forces dispatch" | ~76 us x n |
| div_kappa folded into the density sweep | "Fuse the divergence predictor into the density sweep" | ~30 us x n (one sweep replaces two: ~12.6 us arithmetic + a dispatch boundary) |
| acquire timed apart from encode; one in-flight frame | "Time the drawable acquire apart from the encode", "Hold one in-flight frame instead of two" | 0 us; 13.6 MiB and one display period of latency |

The two fusions claim bit-equality with the dispatches they replace. The
nondeterminism finding above says film hashes cannot prove that; the
proof is the preserved operation order in the shader (mass outside the
dot, length(x) recomputed as r) plus the 24-test suite, which caught a
broken intermediate state of exactly this change (the warm start dropped
from the schedule: three tests exploded).

Rejected by the panel with shown arithmetic, so nobody re-litigates them
without new evidence: a single-workgroup megakernel (loses 900..3,000 us:
five of seven A15 cores idle), f16 solver storage (the solver is latency-
and dispatch-bound, not bandwidth-bound, at 1,620 particles), shrinking
the quarter-res field texture (measured optimum), present-mode changes
(setDisplaySyncEnabled is never called on iOS), merging the field pass
into the swapchain pass (needs framebuffer fetch wgpu cannot express).

### Device runbook for the next cool-phone session

Deploy the head of m3-fluid. Then, in order:

1. Idle gate. Phone flat on the desk, hands off, ~6 s. Expect: `idle`
   climbing ~120 per stats line, `frames` frozen, stats lines arriving
   every ~4 s (30 Hz nap tick). Leave it 5 minutes; record the thermal
   ladder (serious -> fair -> nominal) with times. This is the thermal-
   recovery measurement the fines demand.
2. Wake. Pick the phone up. The response must feel instant; any visible
   freeze-then-jump is a fail (wake budget: two 30 Hz frames).
3. Rest-awake GPU. Hold the phone upright in hand (tremor keeps the sim
   awake), let the pool settle, read `gpu µs p50` at `n 2`. Cool
   baseline before round one: 6,342 us (2026-08-31). The atomics fix
   predicts 3,400..5,700 us.
4. The split timer. At rest expect `acq` ~1,500..1,700 us (display-rate
   back-pressure, healthy) and `cpu` collapsing to ~50..200 us. The O2
   2 ms line now reads against `cpu` alone.
5. Pacing with one in-flight frame. Gentle tilts: `interval p99` must
   hold 8,334 us wherever yesterday's build held it. If it parks at
   16,668 us, revert "Hold one in-flight frame instead of two" alone.
6. Memory. Expect ~13.6 MB below the 86.0 MB baseline.
7. Violent shake, ~10 s. Record `gpu p50/p99`, `n`, `interval p99`.
   Yesterday's hot reference: gpu ~35,000 us at n 16, ~29 Hz. Cool and
   fused, the panel's model says the n=16 solver is ~2,300 us lighter;
   judge the feel, not just the number.
8. Book every delta in the ledger, dated, cool-to-cool only.

A two-minute A/B for the same session, not yet landed: workgroup_size
256 -> 64 on the nine particle kernels (the launch-imbalance cure; 7
workgroups cannot fill 5 cores). Land it only with a measured win.

### Round one on the device: two reverts

Deployed 2026-08-31 15:11, thermal nominal. Two of the four changes
failed on the device within minutes and are reverted; the fusions stay.

1. The aliased velocity-grid view. Jack's recording (15:12) shows faint
   rectangular blocks of stray speckle beside the fluid body, lower half
   of the screen, block size ~the velocity grid's ~108 px cell — stray
   tracers advected by garbage velocities, clustered per cell. The Mac
   tests and films never showed it: the splat-to-advect hazard on one
   buffer bound atomic-write in one bind group and plain-read in another
   is not honoured on the A15, or not by this wgpu on Metal. The panel's
   escape hatch is the way back in: a resolve dispatch (7 workgroups)
   copying the atomic grid to a plain buffer between splat and advect,
   +25 KB, keeping nearly all of the predicted 650..2,950 us. Round two.
2. One in-flight frame. The phone locked at 60 Hz on a nominal battery:
   interval p50 16,668 us, acq p50 12..14 ms, gpu p50 ~11 ms at mixed
   n 2..10 — the predicted no-slack basin. With two drawables, one
   overrun frame (the launch transient suffices) halves the cadence,
   the doubled dt doubles n, and the doubled GPU load keeps it there
   while the phone is being played. Three drawables absorb the overrun
   and recover. The 13.6 MiB goes back on the shelf unless a future
   build's worst frame fits the budget with margin.

Lesson, standing: a change whose risk note names a device-only failure
mode gets a device check before the next feature lands on top of it,
and before the user meets it. The film harness cannot see pacing or
cross-dispatch hazards.

## The physics pass: fix the jitters, budget later

Jack's directive, 2026-08-31: fix the jitters properly, disregard the
frame budget, optimise after the fact. This pass trades microseconds
for accuracy on purpose. Every cost it adds is a named target for the
optimisation pass that follows it.

### The convergence ladder: the boil was timestep error (2026-08-31)

Two latent frame-rate couplings fell first. The substep floor was a
count, so a 60 Hz frame ran 8.3 ms substeps — the length the record
proved non-convergent. The refine depth keyed on the count too. Both
now key on substep length (`DT_SUB_MAX`, `refine_passes`), so film at
120 Hz and a throttled device agree by construction.

The ladder: {4.2 ms cap, 2.1 ms cap} x {shipped refines, 16 refines}
x {upright, reclined, flat}, tripod, NOISE=0.15, raw field, gate off.

| Config | Upright boil mm | Reclined jump mm/frame | Flat stir |
|---|---|---|---|
| 4.2 ms, shipped | 0.42 | 0.077 | 1.62 |
| 4.2 ms, deep | 0.47 | 0.075 | 1.59 |
| 2.1 ms, shipped | **0.14** | 0.074 | 1.38 |
| 2.1 ms, deep | 0.14 | 0.074 | 1.41 |

The verdict is unambiguous. Refine depth changes nothing at either
substep length: the density solve is already converged, and the boil
is the integrator's dt-squared error, not iteration residual. Halving
the substep to 2.1 ms cuts upright boil threefold (0.42 to 0.14 mm),
ends the rest clamp storm (20,705 to 437 over the film), and settles
the post-tilt corner ten times calmer (v_max 0.36 to 0.03). Reclined
was already at its floor. Flat spasms survive every config — the
wall-model error stands as its own workstream.

Shipped: DT_SUB_MAX 2.2 ms. Rest runs four substeps instead of two,
about +1.1 ms GPU at rest on the M3 solver-cost split. The refine
schedule is saturated at 2.1 ms (16 refines buy nothing over 5), so
the optimisation pass can cut refines there with the compr guard.

### Surface tension, priced and re-landed with wetting (2026-08-31)

The earlier verdict — "the tension able to kill millimetre chop is
orders above physical" — was wrong, and the derivation that shows it
also rescues the model. Integrating the Akinci cohesion potential
across a cleaved half-space prices the model's effective surface
tension: sigma = 0.107 * gamma N/m at this support radius (a discrete
lattice sum agrees within 20%), so water is gamma = 0.68 and the old
sweep's 0.5..1.0 straddled physical. The balling it saw was correct
physics: a shallow pool at physical tension on a wall with no
adhesion is water on a superhydrophobic surface, and its equilibrium
puddle height (5.4 mm) exceeds this pool's depth. The glass was
waxed. The missing mechanism was wetting, not a smaller knob.

Re-landed from the preserved diff, rebased over the fused solver:
normals pass (st_normals, one extra neighbour sweep), cohesion +
curvature in forces_eval, and the new piece — wall adhesion through
the same analytic-wall machinery. The Akinci adhesion kernel's
half-space integral J(d) has no closed form; a quadrature-pinned
polynomial in u = 2 - d/h carries it (fit error 0.3%, test:
wall_adhesion_polynomial_matches_the_kernel_quadrature).

Young-Dupre closes the loop and makes ADHESION a contact-angle dial:
the wall's work of adhesion is beta * rho^2 * K (K from J), equated
to sigma (1 + cos theta). At gamma 0.68: beta 2.08 is 110 degrees
(oleophobic phone glass), 3.16 is 90, 4.73 is 60, 5.89 is 30 (clean
glass), 6.31 is 0. The model's own cohesion work at gamma 0.68 comes
out at exactly 2 sigma of water, confirming both anchors.

Filmed across the dial (tripod, NOISE=0.15, raw field, 2.1 ms cap):

| beta (theta) | flat stir | flat jumps >0.3mm | upright boil mm |
|---|---|---|---|
| no tension | 1.38 | 114 | 0.14 |
| 1.36 (~120) | 1.19 | 79 | 0.24 |
| 2.72 (~97) | 0.75 | 12 | 0.31 |
| 4.73 (60) | 0.70 | 39 | 0.26 |
| 5.89 (30) | 0.86 | 119 | 0.27 |

At 30 degrees the water wicks around the whole perimeter and climbs
the corners — real glass-box behaviour — but face spreading stalls
against a stable dry patch: lateral spreading pressure on a face is
an emergent thin-film effect the 2.5 mm discretization barely
carries, where corner wicking gets direct lateral adhesion from the
side walls. Shipped: beta 2.08, the measured contact angle of water
on an actual iPhone screen. The physically honest flat pose is a
beading puddle, not the old full sheet — a 2.4 mm water layer cannot
stay sheeted on any real material — and the hydrophobic end of the
dial is also the calmest measured flat config. The upright boil rise
(0.14 to ~0.26 mm) reads as forced capillary ripple under the film's
sensor-noise shaking; the zero-noise floor check follows the wall
wedge fix.

### The flat spasms were the walls' corner arithmetic (2026-08-31)

A temporal-activity map of the flat film put nearly all the motion on
the box perimeter, not the interior — so the record's z-contact-layer
diagnosis was wrong. The cause is geometric: the wall fill sums six
per-axis half-space integrals, and where two perpendicular walls'
supports overlap it counts the shared wedge twice. An edge contact
particle reads +5.1% of rest density that does not exist (quadrature;
the corner-most seeded layer reads +8.6%). The solve pushes the
phantom compression apart, the fill drops, fluid falls back: the
spasm engine, running on every edge of every pose. The +2.2% contact
bias the record calibrated away with a blunt 0.978 scale averages the
same wedge error over the contact population.

Shipped: inclusion-exclusion to pair order. wedge(t1, t2) is the
kernel's quarter-space integral and wedge_d its partial, degree-6
fits pinned by quadrature tests; wall_grad_sum carries the matching
gradient relief, so the density solve, its divergence predictor and
the refine loop all see one geometry. The 0.978 scale is removed —
the fudge is now derived. The corner-most residual is +3.4%, the
pristine-lattice midpoint bias the half-lattice test already
documents. Left out, recorded: the triple-overlap octant (+1.1% at a
three-wall corner, opposite sign) and the same wedge relief for the
adhesion kernel; both are second-order to what the films show.

### The verdict films, and the curvature cut (2026-08-31)

The full stack — 2.2 ms substep cap, wedge-corrected walls, tension
with adhesion — against the morning's shipped state (tripod,
NOISE=0.15, raw field):

| Meter | Morning | Full stack |
|---|---|---|
| Flat level jumps >0.3 mm / 180 frames | 114 | 6 |
| Flat stir | 1.62 | 0.80 |
| Reclined jumps >0.3 mm | 1 | 0 |
| Upright boil mm | 0.42 | 0.30 |
| Shake compr max % | ~0.15-0.19 | 0.13 |

Upright was the sore number, and a curvature-weight sweep found the
cause in the model itself:

| Curvature weight | Upright boil mm | Zero-noise floor mm |
|---|---|---|
| 1.0 | 0.30 | 0.22 |
| 0.5 | 0.34 | 0.19 |
| 0.0 | 0.14 | 0.08 |

The Akinci curvature term reads colour-field normals, and at 1,620
particles those normals are noise: the term that smooths a dense
surface stirs this one. Cut, with its normals pass and buffer — the
cohesion spline alone is what the cleave integral prices, so the
tension anchor is untouched, and the wetting statics (meniscus,
beading, contact angle) live in cohesion + adhesion. The flat blob's
rim wobbles a little more without it (level-jump mean 0.22 vs 0.11
mm); upright and handheld, the poses Jack lives in, halve their
boil. Curvature can return with resolution.

Zero-noise floor 0.08 mm and noisy 0.14 mm are the best surface
numbers this project has measured. The remaining flat activity is
the physically-correct bead slowly wandering under sensor noise.

Both tension coefficients are now computed from the support radius at
run time (sigma = (21/7040) gamma rho^2 c^2, the 0.8665 lattice
factor, Young-Dupre at 110 degrees), so a spacing change retunes
them instead of silently detuning them. Known debt, extended: like
the near force, the cohesion force does work the temperature ledger
does not book; the dT stat range on device remains the watch.

### The 2.0 mm probe: resolution is no longer the binding constraint

The runtime tension anchors made the probe a pure spacing change.
At 2.0 mm (tripod, NOISE=0.15): upright boil 0.16 mm, zero-noise
floor 0.08 — no better than 2.5 mm's 0.14 and 0.08 — with more
clamp activity everywhere (the clamp ceiling scales with spacing)
and roughly double the solver cost. After the wedge fix and the
curvature cut, the jitter floor is set by the solver's noise
sources, not the discretization. 2.5 mm stays the default, now on
evidence rather than budget.

### The basin the cap dug, and the climb out (2026-08-31)

Jack, on the physics-pass build: looks superb, and moving the phone
takes five seconds to answer. The console shows why: n railed at
15-16 with the fluid at rest (v 0.09), interval p50 29-43 ms. The
substep floor divided the measured interval, so a slow frame
demanded more substeps and made the next frame slower — positive
feedback with two self-consistent states, and the warm phone found
the bad one. The felt five seconds is time dilation on top: the sim
integrates at most MAX_DT (33 ms) per frame while real frames ran
36-43 ms, so sim time fell behind wall time by a fifth, and the
phone's pose ran seconds ahead of the pool's.

The fix keeps the convergence principle and breaks the feedback: the
floor divides the nominal frame (1/120 s), never the measured one —
a slow frame keeps the floor of the frame the display is aiming for.
The CFL term still reads the measured dt, so real violence still
raises n. A second cost bite in the same commit: the cohesion spline
was computing pow(c, 9) per neighbour pair per substep; the pair
loop now hoists both spline constants. Film: boil 0.13 mm,
unchanged. Device numbers follow the redeploy.
