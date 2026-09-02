# The optimisation pass

*Design record for the one aggressive optimisation pass. Binds the
code. Amend by explicit edit. Opened 2026-09-01 on Jack's directive:
"start work on optimisation".*

## Scope and rules

The pass runs once, on the complete frame, after the M4 renderer
(sequencing decision, M4 record). It firms budget O2, sets the
battery bound, and closes the M3 "inside budget" clause. Every
performance number carries the device and the date. A laptop film
proves safety only; benefit is proved on the reference device.

## Targets

| # | Target | Source |
|---|---|---|
| 1 | The 60 Hz substep-floor basin | M4 record, Budget |
| 2 | The solver's per-substep cost (recorded as 50-90 us dispatch overhead; re-measured below) | M3 record |
| 3 | The M3 reclaims: rest substeps (~+1.1 ms), refine schedule | M3 record |
| 4 | Budget O2 firmed; battery bound; frame-latency-1 experiment | HANDOFF |
| 5 | Adaptive resolution: no-go, recorded below | Jack's ask, 2026-08-31 |

## Target 1: the basin, and the substep-length cap

The mechanism (measured 2026-08-31, M4 record Budget): at 60 Hz,
dt = 16.7 ms floors substeps at 8, ~184 dispatches cost more than
the frame, and the loop feeds itself; only the idle gate's sleep
exits. The water can be at rest throughout.

The cap DT_SUB_MAX = 2.2 ms was set 2026-08-31 against resting boil,
before the 4x scale landed. Re-measured 2026-09-01 (film harness,
Jack's laptop, XSPH 6) at 4.2 ms:

| Guard | cap 2.2 | cap 4.2 | Verdict |
|---|---|---|---|
| Rest mean speed, 120 Hz | 0.0176 m/s | 0.0183 m/s | +4%, pass |
| Rest mean speed, 60 Hz | 0.0199 | 0.0209 | +5%, pass |
| Rest compr max | 0.005% | 0.018-0.021% | trivial, pass |
| Ring 120 Hz (env tau / life) | 3.26 s / 7.27 s | 4.16 / 7.31 | pass |
| Ring 60 Hz | 3.46 / 9.17 | 3.74 / 8.23 | pass |
| Shake 120 Hz compr max | 0.034% | 0.080% | pass (2026-08-31 pops: 6.5%) |

The remaining guards, same session: the wake film sleeps at frame
1725 against 1744 (pass); glass flicker 3,603 px/frame against the
12,000 threshold (pass); the 60 Hz shake — the 2026-08-31 pop
scenario — reads compr max 0.201% against the 0.043% anchor, 30x
under the historic 6.5% pops and under the device's own routine
motion numbers (pass).

The cap binds only at low speed: in motion the CFL term sets more
substeps than the floor. At 4.2 ms the floor at 60 Hz halves to 4
(the basin's ~19 ms of work halves; the frame fits, the display can
climb back), and the everyday 120 Hz rest frame halves to 2
substeps. Decision 2026-09-01: DT_SUB_MAX = 4.2 ms ships on film
guards alone; the basin A/B and the settled GPU p50 confirm or
revert it on the device (runbook, below). Rejected: a state-aware
floor that relaxes only at rest — more machinery for a subset of the
same win, worth revisiting only if the device refutes the flat cap.

## Target 2: where the solver time actually goes

Per substep, in order: clear_counts, count_cells, scan_single,
scatter (the grid); density_div, div_apply (divergence solve);
forces_eval, forces_apply; den_apply; then refine_passes(dt_sub)
pairs of den_kappa + den_apply; integrate. 21 at five refine passes.

The per-kernel profile (2026-09-01, film harness host, one timestamped
compute pass per dispatch, 600 rest + 600 shake frames; device numbers
pend) refutes the recorded 50-90 us per-dispatch overhead story. The
gap between passes is ~1 us. The cost sits inside the five
neighbour-sweep kernels, and it is latency, not work: at 1,620
particles the GPU runs 1,620 threads, each walking a long serial
chain of dependent loads.

| kernel | us/dispatch, serial | us/dispatch, lane-parallel |
|---|---|---|
| density_div | 155 | 47 |
| div_apply | 154 | 44 |
| forces_eval | 196 | 57 |
| den_kappa | 147 | 43 |
| den_apply | 153 | 43 |
| clear/count/scatter/forces_apply/integrate | 5-7 | unchanged |
| scan_single | 24 | unchanged |

The fix, shipped: LANES = 8 threads share one particle, each sweeps
a slice of the 27-cell stencil, partial sums reduce through workgroup
memory. Solver GPU total over the profile run: 10.57 s -> 3.24 s,
3.3x. LANES = 16 measured within noise of 8 — the knee. The math is
unchanged up to float summation order, which the solver's atomics
already forgo. The film guard suite passed on the lane-parallel
build (2026-09-01, film harness host). Settled flat mean speed
0.0193 m/s, compr 0.024%. Ring life 6.72 s. Glass flicker 3,648
against the 12,000 threshold. Shake compr 0.057%. The wake film
sleeps at frame 1700. Spin retention tau 5.71/5.03 s against the
serial anchors 5.41-5.47/4.69-4.78: equivalence within scatter.
Every number here is the laptop; the device runbook prices the
change on the phone before any merge.

Dead lever, measured the same night: the refine schedule. Dropping
the five constant-density refine passes to 3 or 2 at rest explodes
the glass flicker meter 3,603 -> 35,002 -> 76,168 px/frame against
the 12,000 threshold, while probe mean speed barely moves — the
pressure-field shimmer is invisible to the speed metric. Five passes
are load-bearing at the 4.2 ms substep; refine_passes stays.

Fusion candidates (superseded in priority by the lane split; the
dispatch gap they would remove measures ~1 us): grid reuse at rest,
forces_eval + forces_apply via velocity ping-pong, integrate into the
next count_cells. Revisit only if the device profile disagrees with
the laptop shape.

## Target 5: adaptive resolution — no-go (2026-08-31, night)

Jack's ask: variable particle density/sizing, "potential to greatly
improve performance for any areas of lower velocity or in the middle
of bodies of fluid." Desk research only: literature survey plus a
repo cost and uniformity inventory. Verdict: no-go at the current
scale. The premise fails three ways in this regime:

1. No middle. The slab seeds two particle layers deep (1,620
   particles); every particle sits within one support radius of a
   wall. A merged particle's h would exceed the slab depth.
2. Low velocity already costs nothing. The idle gate sleeps the sim
   at rest; at rest the solver is ~1.1 ms of the frame.
3. The cost does not scale with particle count. The per-kernel
   profile above re-measures the mechanism: each sweep is
   latency-bound, so halving the particle work leaves the sweep time
   nearly unchanged. In motion the
   finest particle sets the global CFL substep, and the survey found
   no published DFSPH-compatible scheme that escapes this:
   asynchronous regional stepping breaks DFSPH's global solve.

Literature regimes do not transfer: Vacondio's x58 is multi-million
3D dam breaks; Winchenbach's adaptivity ratios live on large open
domains; the DFSPH surface-band scheme replaces a big interior with
a grid, and this slab is already all surface band. No surveyed
source tests under 10k particles in a thin slab; the verdict is
inference from mechanism, flagged as such.

Uniformity inventory, for whenever this re-opens: one scalar h and
mass in SimParams (15 h and 8 mass uses in sim_solve.wgsl), one grid
cell size, tension and adhesion closed-form from the global h, a
uniform seed lattice, meters keyed on one spacing. Per-particle
mass/h means a new storage buffer, every kernel call site, a grid
redesign, and a tension re-derivation. Re-open trigger: particle
count grows ~10x, or a milestone adds a genuinely 3D volume.

## The device session (2026-09-01 morning, reference device, thermal "fair")

Items 1-3 of the runbook closed in one session on the branch head:

1. Jack's eye, verbatim: "yes i can see it swirling now - merge it".
2. The basin is dead. First hard shake: v 2.2, n 13, GPU p50 14.8 ms,
   the display at 60 Hz for one one-second stats window, then back
   to 120 Hz with the water still moving - no sleep needed. Second
   shake: n 11, and the 120 Hz median never dropped; only the p99
   tail degraded. The old build locked at 60 Hz and ~19 ms until the
   idle gate slept.
3. Settled GPU p50 6.15 ms at n 2 (old build 7.557 ms; pre-glass
   6.694 ms): the lane-parallel solver and the two rest substeps
   paid for the whole glass renderer. Compression avg 0.007%, motion
   max 1.35% - the normal device range. The idle gate slept on the
   desk. Caveat: this session ran thermally "fair" against the old
   numbers' "serious"; a few tenths of the delta may be thermal, the
   rest is mechanistic (n 4 -> 2, sweeps 3.3x).

The REVIEW.md device-measurement blocker on the branch cleared with
these numbers; Jack ruled the merge the same morning.

## The cost model after the lanes (laptop, 2026-09-01)

The per-kernel profile, re-run on the merged head (worktree probe:
one timestamped pass per dispatch, 600 rest + 600 shake frames,
first 30 discarded, Jack's laptop). A calibration run first: the
same frames with the whole solver in one timestamped pass read the
solver at 2,097 us per average frame, against 2,568 us summed from
the per-dispatch run — isolating dispatches into their own passes
inflates the total ~22%. Read the table as proportion, not truth.

| Pass | us each (isolated) | Note |
|---|---|---|
| forces_eval | 51 | heaviest sweep |
| density_div | 43 | |
| div_apply | 39 | |
| den_apply | 39 | runs 1 + refine times per substep |
| den_kappa | 38 | refine only |
| reduce_stats | 29 | once per frame |
| scan_single | 26 | one workgroup, serial 32 cells/thread |
| blur_h, blur_v, field_splat, advect | 22-26 | once per frame |
| narrow kernels (clear, count, scatter, forces_apply, integrate) | 4-6 | |
| surface_draw | 2 | laptop-cheap; device res is 4.0x |

Whole laptop frame 2.23 ms average, solver 94% of it. The lanes
took the sweeps from 147-196 us to 38-51 us; what remains above
the narrow-kernel floor is the sweeps themselves, the refine
chain's volume (a measured dead lever — flicker, Target 3), and
scan_single, whose isolated 26 us is an upper bound the shared
production pass may not pay. Laptop-visible candidates, none built
— A15 proportions must pick the target first (the runbook, below):
a parallel scan, a forces_eval split, sweep fusion.

## The captures, mined (reference device, 2026-09-01)

Three console captures banked this day — the morning basin session,
the dye session, and the GUI-free deploy — mined per window
(scratchpad o2mine.py; settled = awake, v < 0.1, n <= 2; motion =
v >= 0.3; sleep windows excluded):

| Number | Settled | In motion |
|---|---|---|
| GPU p50, median window | 6.11-6.46 ms | 6.23-6.75 ms (worst window 14.8 ms) |
| Interval p50 | 8,334 us in every window | 8,334 us in 155/156 windows |
| Interval p99, median window | 8,334 us | 16,668-33,335 us |
| Interval p99, worst window | 17,545 us | 41,669 us |
| CPU encode p50 / worst p99 | ~1.0 / 1.8 ms | ~1.0 / 3.2 ms |
| Footprint | 67-110 MB | same |
| Thermal | nominal-serious | same |

The shape for O2: settled is closed — 120 Hz clean, encode under
2 ms, footprint half the 200 MB line. In the hand the median never
leaves 120 Hz, but p99 spikes to 3-5 dropped frames whenever
sustained handling holds n >= 3 and GPU p50 crosses the 8.33 ms
budget (the GUI-free capture shows acq p50 rising to 7 ms there —
swapchain back-pressure, not basin lock-in; every window kept the
120 Hz median). Firming O2 must either price those motion drops as
accepted or set the next optimisation target to close them; that
call needs the minute hand-test.

Two confounds bind the next session's numbers. Every capture above
ran plugged in and charging (battery columns read 85 -> 90% —
useless for drain). And the GUI-free build deleted the overlay's
~100 Hz main-thread SwiftUI re-render, so CPU-side baselines move
for a reason no measurement isolates (review finding, 2026-09-01);
GPU numbers should not care. The one overlay-free capture is
also the one reading the highest settled GPU p50 (6.46 ms — but
six windows, plugged and charging): re-measure settled p50 clean
next session before reading anything into either fact.

## The device profile, and what the A15 actually measures (2026-09-01, night)

The instrument is a worktree profiler (scratchpad `prof/profdev_patch*.py`,
`profdev.sh`, `profmine.py`; never in the repository): per-pass timestamps
at the production pass boundaries, a fine mode with one pass per dispatch,
`FLUID_IDLE=0` (gate off), `FLUID_NMIN` (pinned substeps),
`FLUID_REPLICATE` (the whole frame encoded R times), `FLUID_SKIP`
(field, fill, points draws left out), `FLUID_SPACING` (the particle
ladder). Three facts about the instrument bind every number below.

1. The governor. The stage-0 finding stands: below saturation the GPU
   span is the clock the governor chose, not the work. Replication
   saturates it, and the display cadence under replication is the one
   honest whole-frame meter: a frame that holds 8,334 us at R = 4 costs
   at most 2.08 ms.
2. Spans overlap. Metal starts an encoder before the previous one drains
   when no resource hazard forbids it, and a stage-boundary timestamp
   fires at the start; a pass that then waits on a hazard reports the
   wait. Per-pass sums therefore exceed wall time (31 to 45 ms of spans
   in a 16 to 20 ms frame at R = 4), light passes that follow heavy ones
   read high, and the fine mode adds an encoder boundary to every
   dispatch. Read per-pass numbers as an ordering and as an upper
   bound; compare only like against like.
3. The session. Jack handled the phone through most runs (v_max 0.3 to
   2.3 m/s), the thermal state climbed from nominal to serious and
   stayed there, and the phone was plugged and charging. No number here
   is a resting, cool measurement.

### The baseline, fine mode (opt-remainder head, pinned n = 2, R = 4)

| Pass | us each | Note |
|---|---|---|
| den_apply, den_kappa, div_apply, density_div, forces_eval | 135 to 249 | thirteen sweeps a substep, ~2.1 ms of ~2.3 |
| clear_counts, count_cells, scatter, forces_apply, integrate | 3 to 7 | a dispatch boundary is single-digit us on the A15; the old 50 to 90 us was the governor |
| scan_single | 26 to 38 | one workgroup, serial |
| surface (fill + points, one pass) | 2,300 to 3,100 | per frame |
| blur_h, blur_v, field_splat | 180, 210, 145 | per frame |
| advect, splat_vel, clear_vel, resolve_vel, reduce_stats | 185, 60, 40, 25, 30 | per frame |

The solver owned ~57% of the frame and the surface pass ~28%; the
laptop table's 94% solver was the laptop's tiny render, as the record
warned.

### The changes, each with its guard and its device number

Branch `opt-sweeps`, stacked on `opt-remainder` on `no-gui`. Every
commit is gate-green and passed the film guard suite (spin, flat
settle, upright flicker, ring, shake, wake; same-night baseline, the
wideguard2 recipe) within run-to-run scatter. Two guard notes: the
upright film's first-second clamp count is bimodal at seed on every
build (20 or ~640; measured six times on the pre-list build), and the
flicker meter reads 9,600 to 11,000 on the head against 9,575 to 9,890
on the baseline over four runs each — inside the recorded band, under
the 12,000 threshold, and first seen at the list commit, so it is the
list's changed summation order, not the filter's half floats. Watch
it.

1. The clear dispatches and the force step (98c54e0). scatter zeroes
   the counts the scan consumed and the sweeps read a cell's end as the
   next cell's start; resolve zeroes the tracer grid it copied; the
   force step fused into the first constant-density apply with the warm
   start in the forces sweep's epilogue; chargeless tracer dots parked
   outside the clip volume. Measured cost of what left, from the
   baseline fine table: 3.1 + 6.9 us a substep and 35 to 44 us a frame.
   Small; kept for the simpler substep.
2. The neighbour list (7e2164f). The density sweep walks the stencil
   once and writes each particle's true neighbours with their kernel
   gradients (the stencil block spans ~6x the kernel's sphere, so most
   old pair evaluations were zeros after three dependent loads); the
   other twelve sweeps read the list; the wall gradient sum is cached;
   kappa is carried as kappa over density. Cap 96 with a counted
   overflow (`nbr` in the stats line; zero in every film and every
   device window). Fine mode, same method both sides: den_apply 173 ->
   24 us, den_kappa 176 -> 27, div_apply 135 -> 24, forces_eval 186 to
   249 -> 63, density_div 144 to 174 -> 175 to 183 (it builds the
   list). Per substep, isolated sums, ~2,300 -> ~590 us. Gate off,
   handled, n 3 to 5: interval p50 = p99 = 8,334 us with acquire p50
   0.9 ms (baseline under handling: acquire 7 ms, p99 25 to 33 ms).
3. The field filter (ce39dde). One compute dispatch blurs both
   directions in workgroup memory and writes the blurred thickness with
   its raw texel differences, first and second; the surface shader
   takes one sample instead of five. Bilinear sampling commutes with
   the difference stencil, so the optics are unchanged up to the
   half-float store. Resting-phone protocol (below): no measurable
   whole-frame change against the list head — moving state 11.86 and
   11.88 ms against 11.90 ms per R = 4 frame, and the still state
   locks 120 Hz on both. The review found the first cut stored the
   Laplacian already divided by step², which overflows the half-float
   store at the waterline (1/step² is about 1.3e6 on the device);
   74bd175 stores the raw second difference and divides in the fill,
   with a test that recomputes every interior texel's differences
   from the stored thickness (drift 0.012 against a 0.03 bound).
4. Subgroup folds and the scan chunk (08d2f15). The lane folds are
   three `subgroupShuffleXor` steps with no barrier and no workgroup
   memory (naga needs no enable directive; `Features::SUBGROUP` is
   required unconditionally, as IMMEDIATES is); the scan's serial chunk
   is sized to the grid, which also lifts the 8,192-cell cap.
   Resting-phone protocol (below): moving state 11.86 ms -> 11.01 and
   11.31 ms per R = 4 frame, 0.15 to 0.22 ms a production frame (5 to
   7%); the still state locks 120 Hz on both.

Whole frame, cadence under R = 4 at pinned n = 2, thermal serious:
baseline 20.75 ms -> 5.2 ms a frame; head (cuts + list + filter) 16.5
ms -> 4.1 ms; head with every render draw skipped locks 8,334 us ->
under 2.1 ms a frame for solver plus tracers. The same skip on the
baseline reads 9.2 ms -> ~2.3 ms, which says the baseline's isolated
solver sums overstate it: encoder boundaries cost more on the A15
than the laptop's 22%.

The resting-phone protocol (2026-09-01, 22:50 to 23:10; phone on the
desk, plugged in, hands off, thermal nominal to fair): each build
installed and run 30 s at R = 4, pinned n = 2, gate off, 60 s idle
between runs, in mirrored order (list, filter, head, head, filter,
list). The meter is the mean interval over each 120-frame window,
from a cadence line added to the worktree profiler: the median
interval quantises to the vsync and cannot resolve a change smaller
than one. The windows split by the fluid's own state. While the seed
transient or the boil moves it (v_max at or above 0.4 m/s) an R = 4
frame costs 11.0 to 11.9 ms; once it stills (v_max under 0.1) every
build locks 8,334 us, so a still production frame costs under 2.1 ms
on all three and the meter floors. Compare like states only, and do
not read the production GPU span in the still state: the display is
not saturated there and the span is the governor's clock again.
Moving-state medians per R = 4 frame: list 11.90 ms (14 windows),
filter 11.86 and 11.88 (16, 14), head 11.01 and 11.31 (10, 4).

The draw attribution, same protocol, head build, R = 6 pinned n = 2,
30 s a run, moving-state medians per R = 6 frame: every draw 15.5 ms;
without the fill 15.9 (no change: the fill hides behind the next
replica's compute); without the points 12.3 (the tracer dots cost
~0.5 ms a moving production frame, and nothing at rest, where the
chargeless dots park outside the clip volume); without field, fill
and points the cadence locks 8,334 us in both states, so solver plus
tracers cost under 1.4 ms a frame. The render side of a moving frame
is therefore ~1.2 ms of ~2.6, and the fine split's 2.3 to 3.1 ms
surface span was overlap, not cost.

### The review's findings (2026-09-01, night)

A fresh-context review of the branch (top model; the diff, the
repository and REVIEW.md, never the author's rationale) found two
defects and one gap, all closed in 74bd175 and this record:

1. scatter's counts-zeroing emptied the stage-0 microbench: its
   density sweep read a cell's end as start plus count, so
   `FLUID_BENCH` timed an empty loop. The sweep now reads the count
   from the scatter cursor, and the bench's first-frame validation
   checks every sixteenth density against a brute-force CPU sum
   beside the scan; a test runs the same function (32 tests). The
   bench's kernel is the old 27-cell stencil, not the shipped list
   sweep, so it no longer times the shipped path; whether it stays
   is Jack's call.
2. The Laplacian overflow, item 3 above.
3. The device numbers, items 3 and 4 above.

Advisories taken: the stale sweep comments; the feature-gate comment
(wgpu grants SUBGROUP on Metal 3 GPUs, A13 and later — the iOS 17
floor admits the A12, which would fail device creation; raise the
floor or accept the exclusion, Jack's call); the workgroup array
sizes as constant expressions of their dials. Left as is:
`nbr_over` accumulates for the run, as `clamp_count` does.

### The ladder (list build, gate off, pinned n; the cadence is the meter)

| Spacing | Particles | Cells | n = 2 | n = 6 |
|---|---|---|---|---|
| 0.010 (shipped) | 1,620 | 1,568 | 120 Hz | 120 Hz held under handling (above) |
| 0.008 | 2,584 | 2,380 | 120 Hz | — |
| 0.0063 | 6,336 | 4,515 | 120 Hz | 60 Hz |
| 0.005 | 16,775 | 7,020 | 60 Hz | 30 Hz |

Jack's stated goal (2026-09-01, verbatim): "the ultimate goal here,
btw, is to greatly increase the simulation size to 4x or 8x or even
16x the number of particles". The solver spans scale close to
linearly with particle count (per-substep spans 0.55, 1.3, 1.8 to
1.9, 4.6 to 4.7 ms up the ladder, overlap caveat applied equally). 4x
particles already rest at 120 Hz on this build; in motion the substep
count also grows with 1/spacing (CFL), so 16x at 120 Hz is out of
reach of this GPU by physics, not code. The render side is a fixed
cost that comes straight out of the particle budget: the surface pass
is 2.3 to 3.1 ms a frame in every capture, and the fine split puts
the 131,072-point tracer draw at 1.5 to 2.6 ms of it (the split's own
tile load and store inflate the upper figure) against ~1.4 ms for the
liquid-glass fill.

### Next, in order

1. The tracer draw: a compute splat of the dots into a full-resolution
   intensity buffer read by the surface shader (exact for single dots
   a pixel; overlapping dots would take the brighter, a look change
   for Jack's eye), or fewer tracers (Jack's dial). Worth ~0.5 ms of a
   moving frame by the draw attribution above, nothing at rest.
2. The builder sweep, now the largest solver item: sort particle state
   into cell order each substep so the stencil walk and the list reads
   are contiguous; then a symmetric build (half the candidate pairs).
3. The refine chain (ten list sweeps a substep): a convergence-gated
   iteration count through indirect dispatch, judged by the flicker
   meter before any device build; LANES 4/8/16 and workgroup 64/256
   as one-constant device A/Bs now that the folds are barrier-free.
4. The runbook items below still stand.

## The runbook's remainder (waits for a phone session)

Session prep, first plug-in: enable network debugging for the
reference device (Xcode, Connect via network) — wireless devicectl
then captures unplugged sessions, which the battery bound needs.

1. The M3 exit measurements:
   - Settled upright hydrostatics: prop the phone upright, hands
     off >= 60 s, read the settled windows before sleep; compare
     the pressure ceiling against rho g times the resting fill
     height at WORLD_SCALE.
   - The minute hand-test: 60 s of ordinary play in Jack's hand;
     record the interval distribution, GPU, compression; read the
     M3 "convincing slosh inside budget" clause against it.
2. Budget O2 firmed from 1; the battery bound from an unplugged
   10-minute play session over wireless capture (or, without
   wireless: plugged stats line, unplug, play, replug, difference).
3. The frame-latency-1 experiment: flip
   `desired_maximum_frame_latency` 2 -> 1 (render.rs, one line) in
   a worktree deploy; A/B the minute hand-test and the settled
   windows against 2; Jack's hand judges feel, the capture judges
   drops. Each latency step is one drawable (~14 MB) and one frame
   of sensor-to-photon lag.
4. The device per-pass split: done, with the worktree profiler and
   the resting-phone protocol above.
