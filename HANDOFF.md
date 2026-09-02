# Handoff — project state

*Updated 2026-09-02. Audience: the next agent, or Jack. This file is the
state document. It holds what the next stretch of work needs. Update it in
the same commit that closes a milestone or a task. Git holds the history.*

## Glossary of the codenames

| Prefix | What it names | Where the series lives |
|---|---|---|
| `M0` to `M7` | A milestone | The roadmap below |
| `Dn` | A binding design decision | `docs/design/decisions.md` |
| `On` | An open decision, binding on nothing | The open decisions below |

## Where things stand

| Item | State | Where |
|---|---|---|
| Stack, shell, frame and units, M1 dependencies (D1 to D4) | Decided 2026-08-30; D1–D4 amended the same day when the web target was removed | `docs/design/decisions.md` |
| Toolchain: Rust 1.98.0 pinned, both iOS targets, cbindgen 0.29.4, Xcode 26.6 with the iOS 26.5 platform | Done 2026-08-30 | `rust-toolchain.toml` |
| The gate | Green 2026-08-30 (8 tests) | `scripts/gate.sh` |
| The web target | Removed 2026-08-30, Jack's call, mid-M1. Git history holds the code; the D1 amendment holds the record. | `docs/design/decisions.md` |
| M0 toolchain slice | Done 2026-08-30: Rust-computed body force on the phone, Jack confirmed the readout | — |
| M1 surface | Done 2026-08-30 on the reference device; measurements below. | `docs/design/m1-surface.md` |
| M2 particles | Done 2026-08-30 by Jack's call: visuals landed and hand-tested; the budget ramp was cut with the pivot to M3. Measurements below. | `docs/design/m2-particles.md` |

M3 state, 2026-08-31: the full DFSPH solver runs on the device — grid
rebuild, wall-corrected density and factor, divergence-free and
constant-density solves, Morris viscosity, temperature, dynamic
substeps. Verified live: compression avg 0.017% max 0.63% in motion,
pressure 0..475 Pa, temperature within microkelvin, 120 Hz held.
Numbers and the two test-pinned defects are in the M3 record. The warm-started density solve ended the at-rest flicker (settled v
0.03 m/s, clamps down 96%). The fluid draws as 262,144 one-pixel
tracers advected in the solved field — Jack's directive, 2026-08-31.
Left for M3 exit: the settled upright measurement and the minute
hand-test with its interval measurement. Surface tension, the third
item that stood here, landed the same day (below).
The ramp is measured (M3 record): 2.5 mm holds 120 Hz clean and
stays the default; 2.0 mm waits on the optimisation pass. Debug on
the desk first: scripts/film.sh films the sim to an mp4 without the
phone.

2026-08-31, the jitter campaign closed and the efficiency push opened.
Shipped, in order: near-pressure, the substep floor with the low-n
refine boost, the sensor force filter, and the idle gate. The cost
pass landed two solver fusions and the acq/cpu timer split; two other
changes failed on the device and were reverted the same hour. The M3
record holds all of it. Metrology films must set IDLE=0 or the gate
freezes the measured window.

Later that day, the physics pass: Jack's directive, fix the jitters
properly and disregard the frame budget; optimisation reclaims the
cost after. Shipped, each with film oracles and tests in the M3
record: the substep cap re-expressed in substep length (2.2 ms — the
resting boil was timestep error, not iteration residual), the wall
corner-wedge correction (the flat spasms were double-counted fill on
every box edge; the 0.978 fudge is now derived geometry), and Akinci
surface tension — cohesion plus wall adhesion, both coefficients
derived at run time from the support radius, anchored to real water
(sigma via the cleave integral, contact angle 110 degrees via
Young-Dupre, the measured angle of water on a phone screen). The
curvature term is cut as measured noise. Film verdict: flat spasms
114 -> 6 jumps, upright boil 0.42 -> 0.14 mm, zero-noise floor
0.08 mm, reclined dead calm, shake violence intact. The named
reclaim targets for the optimisation pass: the extra rest substeps
(~+1.1 ms GPU) and the refine schedule (saturated at 2.1 ms).

That evening, the jelly investigation (M3 record, "The jelly, taken
apart"): Jack's recording showed jelly creep, a scalloped rest edge,
and residual jitter. XSPH damping is now a rate (48/s), so pacing
policy can no longer move it; TILT and RING poses plus ring-down and
creep meters joined the film harness. The autopsy pinned the jelly on
wall-adhesion contact-line dissipation (ring 0.25 s with it, 0.50 s
without, solver floor 1.0 s, real water 3-6 s here). Gamma reduction,
K_NEAR softening, and viscosity removal are measured dead ends — each
buys ring only by wrecking flat calm. The 2.2 ms cap survived its
damping-confound re-check and stays. The side-wall-only adhesion
that followed (the meniscus without the stickiness) doubled ring
life and tripled tilt response, but on the device it unmasked the
solver's rest-state noise as visible dancing (M3 record, "The
dancing"): full face grip is the only found suppressor, and the
grip is the jelly. Reverted the same night on Jack's call, and the root fix he set as
the open task landed the same night (M3 record, "The noise,
found"): the dancing was input noise, not a solver defect. The
still phone's real sensor noise is 0.02-0.08 m/s^2 (measured on
the device; the harness's 0.15 was a handled-launch misread), and
the force filter's still-phase floor now sits at 0.02 instead of
0.1, which takes the de-jelly geometry from 100-235 dance flips to
4 at the measured noise, with ring, tilt, shake and tremor guards
intact. Two things remained open from that night. First, Jack's
call: grip versus side-walls, now with honest numbers (grip +
filter dances 0; side-walls + filter dance 4 and keep the doubled
ring life and free tilt creep). Second, a defect found on the way:
a sigma-independent rest v_max floor (0.056-0.107, present at
NOISE=0) had starved the idle gate at 1x since tension landed —
the WAKE film's sleep oracle failed in every 1x configuration and
the 1x desk phone never slept. Invisible in the picture (the
movers are a few particles, plausibly circulating at the contact
line). The scale change below ended the starvation without
explaining the floor. The pacing and cost picture is unchanged;
sustained 120 Hz still waits on the optimisation pass: a hard shake
drops the frame into a 60 Hz substep-floor basin that only the idle
gate's sleep exits (measured 2026-08-31, M4 record Budget). The pass
is sequenced after the M4 look notes (the M4 record holds the decision).

2026-08-31, the scale: Jack asked whether the feel was locked
behind a non-1:1 scale, and the measured ladder (1x/2x/4x/8x; M3
record, "The scale, chosen") said yes. The world now models a tank
4x the device — WORLD_SCALE in lib.rs; the sensors still feed
real m/s^2. Jack's verdict on the device build, verbatim: "4x
feels right - lock it in." At 4x: slosh 1.47 Hz, ring-down 6.5 s
(13x the 1x life), tilt unpinned, rest dead calm, 120 Hz
locked at gpu p50 6.27 ms — and the idle gate sleeps on the desk
(settled v 0.02, under V_SLEEP), because the 1x rest-velocity
floor does not scale with the world. The 1x floor stays recorded
and unexplained; its hunt is academic while 4x ships.

2026-08-31, the geometry: the grip-versus-side-wall call is settled.
Re-measured at 4x with identical film suites (M3 record, "The
geometry, settled"), the 1x trade has dissolved: ring, kick, tilt
and dance are equal within scatter, and both configurations pass
the WAKE sleep oracle — the first pass since tension landed. Jack
ruled: keep full grip. Real water wets all six faces, and the cost
is now zero on every meter.

2026-08-31, the render direction: Jack locked the M4 look — a
liquid-glass water renderer (real refraction, Fresnel, a specular
that tracks real tilt) over a procedural black-and-white dazzle
backdrop. The M4 record (`docs/design/m4-water.md`) holds the
directive verbatim, the optics model, the new stripe-flicker meter,
and the sequencing decision: renderer first, then one aggressive
optimisation pass on the complete frame. Jack also asked to
investigate adaptive particle resolution; the findings feed the
optimisation-pass design.

2026-09-01, the rotation and the sink: Jack proved the water never
swirled, and the cause was structural — the sim read only the
accelerometer, and a uniform body force has zero curl. The gyro is
now wired end to end (MotionSample.rotation_rate, the fictitious
force triple in the solver, gate wake on spin, the SPIN film pose);
O6 is closed. The dissipation audit that followed (M3 record, "The
sink, measured") killed the recorded wall-restitution lever and
vorticity confinement with direct measurement, priced XSPH 48 at a
third of all eddy damping, and shipped XSPH_RATE 6 — retention tau
3.5 -> 4.9 s against a measured discretization ceiling of 5.5-6 s
at this particle count. Guards green; the recorded cost is ~2 s of
extra sleep latency after motion.

Test baseline: 45 Rust tests pass, 2026-09-02.

## M1 measurements (reference device, 2026-08-30, Release)

~96 s sustained, captured over `devicectl` console. 120 Hz locked: frame
interval p50 = p99 = 8,334 µs, steady-state max 16,668 µs (a rare single
dropped frame), ~2 s startup transient. CPU encode+submit+present p50
~1.4 ms (includes drawable-acquire back-pressure). `phys_footprint`
63.7 MB. Battery 100%, thermal nominal throughout. The adapter offers no
`TIMESTAMP_QUERY`; GPU pass time reads 0 until a finer probe exists.
Idle verified: stats freeze when backgrounded, resume on foreground.

## M2 measurements (reference device, 2026-08-30, Release, 50,000 particles at 0.6 mm)

From Jack's hand test via the on-screen stats; the minute-long ramp
protocol was cut with the pivot to M3. Interval p50 8,334 µs (120 Hz),
p99 16,668 µs: the settled pile drops occasional frames. GPU pass p50
5,275–5,710 µs. CPU frame path p50 1.7 ms spread, 6.1 ms piled (drawable
back-pressure). `phys_footprint` 78–83 MB. Battery 100%, thermal nominal.
The stats call costs 102 µs once a second, off the frame path. GPU
timestamps read real values on this device in M2; the M1 note that the
adapter lacks `TIMESTAMP_QUERY` did not hold — trust the M2 observation.

## The next task

2026-09-02: the optimisation pass's second stretch merged to master
on Jack's ruling ("Merge this work"): no-gui, opt-remainder and
opt-sweeps fast-forwarded as one line, the branches deleted.

The same day, M5 opened with the menu Jack asked for. The M5 record
(`docs/design/m5-menu.md`) holds the directive verbatim and D6 the
split: a tap shows a button, the button opens a half-sheet menu with
the four particle scales (0.25x, 1x, 4x, 16x, rated good, good,
borderline, bad from the measured ladder), the flat look (two
colours only, black and the chosen colour, hot pink by default, with
a particle view that draws the particles alone as discs, sized by
how crowded each particle is, and builds no field at all) and three readout toggles (frame rate, thermal
state, GPU time against the 8.33 ms budget). Every choice persists.
The core rebuilds the sim at a new scale in 25 ms and carries the
look as a vec4 at byte 48 of the optics immediates; the
settled-field calibration now scales with the spacing, pinned by a
second plateau test. Measured the same
day: the shader edit is neutral at 1x, 4x holds 120 Hz in the hand
and dips to 40 to 60 Hz under a hard shake, 16x sits in the substep
basin at 120 ms a frame. The lenses of the roadmap's M5 row are
still to come, behind the same menu.

Later the same day, two more of Jack's asks landed on the same
branch. First, the drag: a finger on the glass entrains the water it
crosses — towards the finger's own velocity, never away from it —
inside a disc of 25 mm of glass through the whole slab, with all five
of the phone's fingers dragging at once. The shell reports a slot and
a point on its own drawable; the core does every metre. Second, the
lenses: the flat looks take one colour or two, and two make a ramp
across velocity, acceleration, pressure, proximity or temperature.
Every field but acceleration was already in a buffer; acceleration
is the substep's whole velocity change, measured in `integrate` and
carried in the free w of the velocity record. The M5 record holds
the models, the dials and the derivations.

Measured the same evening (particle view, 1x, still desk, per-launch):
GPU p50 settled 2,439 µs before the drag, 2,240 with it, 2,248 with
the lenses on top. Both features are free. Getting there cost a hunt:
the lenses first read 2,723, and four hypotheses died before a
throwaway build that split the compute and render timestamps apart
named the render pass in one run. The cause was one flat `vec3`
varying on the disc draw — 490 µs a frame at 1,620 discs. Both quad
passes now carry the ramp in clip z, which has no depth attachment
to want it. The glass look, measured the same way, pays 76 µs of its
6,428: the field texture's second channel, written once and read once.

What remains, in the order the records list it: the optimisation
record's next steps (the tracer draw, the builder sweep, the refine
chain); the runbook's remainder (the M3 exit measurements, budget O2,
the battery bound, the frame-latency-1 experiment); and Jack's dials
on the drag and his eye on the five lenses. The next one is Jack's
pick.

The history of the pass, for the record:

m4-water merged to master 2026-09-01 on Jack's ruling ("yes i can
see it swirling now - merge it"). The branch carried the whole M4
liquid-glass renderer, the gyro rotation physics, the dissipation
audit with XSPH 6, and the pass's first night: the per-kernel
profile that found the solver latency-bound (not dispatch-bound),
the eight-lane neighbour sweeps (solver 3.3x on the laptop), and
the 4.2 ms substep cap. Device-confirmed the same morning: settled
GPU p50 6.15 ms (old 7.557; pre-glass 6.694), the 60 Hz basin dead,
Jack's eye on the swirl. The optimisation record holds it all.

What remains of the pass (its record, "The runbook's remainder"):
the M3 exit measurements, budget O2 firmed, the battery bound, the
frame-latency-1 experiment. From here work branches off master with
a topic slug (CLAUDE.md section 8).

The churn visibility landed 2026-09-01, by iteration against Jack's
eye on the device. The milk (a dye texture the optics read as
scatter) was built, priced, and deleted on his ruling; what shipped
is the charge memory — each tracer remembers the fastest speed it
recently felt — driving the strand draw through a square-root
response, so gently swirling threads read where they used to be
near-black. The M4 record ("The dye, designed") holds the model,
the dial history, and the verdict verbatim.

2026-09-01, the shell went GUI-free on Jack's directive: the debug
overlay is deleted (git holds it), the status bar and home indicator
hide, and the screen is only water. The stats line still prints once
a second; `devicectl` console capture is now the only measurement
channel. (2026-09-02: the screen is still only water until a tap;
the menu and the readout are the M5 record's.)

2026-09-01, night: the pass's second stretch, on branch `opt-sweeps`
(stacked on `opt-remainder` on `no-gui`; none merged). A worktree
profiler measured the A15 pass by pass and found the settled span is
the governor's clock, that Metal overlaps encoders so per-pass spans
are upper bounds, and that the solver's thirteen neighbour sweeps
owned the substep; the surface pass's flat 2.3 to 3.1 ms span was
overlap, the render side ~1.2 ms of a moving frame and nothing at
rest. Shipped on the branch, each gate-green and film-guarded: the
clear dispatches deleted and the force step fused; a per-substep
neighbour list with cached kernel gradients (sweeps 135 to 249 us ->
24 to 63 us each; the frame holds 120 Hz under handling at three to
five substeps); the blur and derivative work fused into one compute
pass with a one-sample surface shader; subgroup lane folds and a
grid-sized scan chunk. A fresh-context review found the microbench
emptied by the scatter change and the stored Laplacian overflowing
its half floats; both are fixed with tests. The resting-phone
protocol (cadence under 4x replication at two pinned substeps, phone
on the desk) measured the pass: a moving production frame 4.5 -> 2.8
ms, and at rest the head locks 120 Hz (under 2.1 ms a frame) where
the baseline, at two pinned substeps, never stills; the filter is
neutral and the subgroup folds are 5 to 7% of a moving frame. Jack's
goal is 4x to 16x the particles; the measured ladder rests 4x
(6,336) at 120 Hz today. The optimisation record holds the numbers,
the caveats and the next steps.

M3 close waits on the optimisation pass: the roadmap's closure rule
needs the oracle measured on the device, and three measurements are
still unrecorded — the settled upright hydrostatic numbers, the
minute hand-test with its interval measurement, and the "inside
budget" clause. All three fit the pass's device runbook. Closing M3
earlier is Jack's call and takes an explicit closure-rule amendment.
Jack's M3 directive stands verbatim in the M3 record.

## Open decisions

Binding on nothing. Decide each when its milestone needs it, then move it
to `docs/design/decisions.md`.

| Id | Question | Where it is decided |
|---|---|---|
| O1 | Decided 2026-08-30 → D5: DFSPH in a quasi-3D thin slab. Driven by Jack's accuracy directive; PBF and MLS-MPM rejections are in the record. | `docs/design/decisions.md` |
| O2 | The performance budget at 120 Hz on the reference device. Provisional, from M1: frame interval p99 ≤ 8,400 µs sustained; CPU encode p99 ≤ 2 ms; footprint ≤ 200 MB; thermal nominal over ten minutes. Amended 2026-08-31: the old "CPU frame path" number was ~97% drawable-acquire block (swapchain back-pressure), not encode work; the stats line now reports the two apart (`acq` and `cpu`), and the encode figure alone carries the 2 ms line. Firmed in the optimisation pass. | Optimisation pass |
| O3 | The name. "Fluid Box" is a working title; the iOS target is `FluidApp`, bundle `com.screendead.FluidApp`. | Jack |
| O4 | Moot 2026-08-30: the web target is removed (D1 amendment). | — |
| O5 | The license. `Cargo.toml` says `UNLICENSED` until Jack chooses. | Jack |
| O6 | Closed 2026-09-01, ahead of M7: the gyro is wired end to end and the fictitious triple runs in the solver (M3 record, "The rotation, missing"). | `docs/design/m3-fluid.md` |

## Roadmap

Each milestone has an oracle. A milestone is done when its oracle passes on
the reference device and `HANDOFF.md` records the measurement.

| Milestone | What it builds | Oracle |
|---|---|---|
| M0 Toolchain | Body force from Rust on the phone | The readout agrees with CoreMotion; signs verified on the phone. **Done 2026-08-30.** |
| M1 Surface | wgpu owns a `CAMetalLayer` from `fluid-core`; clear and present at display rate; frame-time capture | 120 Hz stable, idle draw measured, budget O2 set. **Done 2026-08-30.** |
| M2 Particles | GPU particle buffer, integration under the body force, box collision, point rendering | Tilt and push the phone; particles behave; particle count at budget recorded |
| M3 Fluid | The method from O1: neighbour search on the GPU, incompressibility, viscosity | A convincing slosh inside budget; incompressibility measured. Open pending the exit measurements (the closure-rule clause). |
| M4 Water | The default view: the liquid-glass renderer from the M4 record — thickness from the splatted field, normals, refraction, the dazzle back wall | Looks like water; better than real time; inside budget |
| M5 Lenses | The menu (opened 2026-09-02, M5 record) and, behind it, the field lenses: velocity, density, acceleration, pressure; temperature as an added field | Each lens switches with no frame drop |
| M6 Headroom | Adaptive substeps, sleep when still, thermal response; power measured | Battery draw recorded against a target |
| M7 Feel | Sensor-to-frame latency measured and tuned; haptics; rotation landed early (O6, closed 2026-09-01) | Latency number recorded; Jack's hand says it feels right |

Jack's directive, 2026-08-30, binds M4 and M5. His words: "the various
different lenses (e.g. velocity, density, acceleration, temperature,
pressure, etc) should be behind a dropdown menu. the default should be as
photorealistic of a water renderer as possible while being
better-than-realtime rendering." The water renderer is the default view;
every field lens sits behind a dropdown.

## Rules that bind future work

CLAUDE.md holds the rules. Three to hold in memory:

- Do not write scaffolding for a later milestone. Add a dependency when
  its first caller arrives.
- The comment rule and the performance rule are Jack's words. Quote them,
  do not paraphrase them.
- Every performance number carries the device and the date.

## Pointers

- `docs/design/decisions.md` — D1 stack, D2 shell, D3 frame and units,
  D4 dependencies; each amended 2026-08-30 for the web removal. D5
  the method, D6 the menu's shell/core split.
- `docs/design/m5-menu.md` — the menu, the particle ladder, the flat
  look, the readout, and their device numbers.
- `scripts/gate.sh` — the whole gate; CI runs the same steps.
- `platforms/ios/project.yml` — the Xcode project source; never edit the
  generated project.
