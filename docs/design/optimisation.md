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

## Where the device numbers were taken (amended 2026-09-03)

Every device number in this record comes from one place. Jack's words,
2026-09-03: the phone lies "mostly flat" beside him on a bed, "as flat
as i can get it", on the cable. He moves, and so does the bed. Some
runs he handled on purpose, "swirling it and so on". Sections that say
"the desk" or "hands off" mean this bed, and mean the water read
still, not that nobody touched the phone.

Two consequences bind every measurement here. A bed sheds no heat, so
the phone throttles sooner and further than a hard surface would: read
an absolute millisecond as an upper bound. A perturbation raises
v_max, which raises the substep count, so a 120-frame window is pooled
by the count it actually ran and a window the pin does not own is
dropped. A cost taken from a difference inside one run survives both
effects. A cost taken from two separate runs survives neither.

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

Whole frame, cadence under R = 4 at pinned n = 2 (2026-09-01, 21:40
to 22:10, handled, thermal serious):
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

Baseline against head under the same protocol (2026-09-02, 10:07 to
10:15; phone on the desk, plugged in, hands off; thermal nominal to
fair; 30 s a run in the mirrored order baseline, head, head,
baseline):

| Build | Runs | Fluid state | Per R = 4 frame | Per production frame |
|---|---|---|---|---|
| baseline, 4d566e4 | 2 | moving, v_max 0.5 to 1.1 m/s for all 30 s | 17.93 and 17.99 ms | 4.5 ms |
| head, 74bd175 | 2 | still from the third window (27 windows each) | 8,333 and 8,332 us, the vsync lock | under 2.1 ms |
| head, 74bd175, 2026-09-01 night | 2 | moving, v_max 0.5 to 1.7 m/s | 11.01 and 11.31 ms | 2.8 ms |

The states differ with the substep the pin takes from the wall
clock. At 17.9 ms a frame the baseline's two substeps are 9 ms each,
twice `DT_SUB_MAX` (4.2 ms): its CFL clamp fires 730 to 800 times an
R = 4 frame and the fluid boils for the whole 30 s. The head at 8.3
ms sits on the bound, clamps under ten times a frame, and rests. The
like-state comparison is the moving state: 4.5 -> 2.8 ms a production
frame, 1.6x, resting and cool, with the head's boil the harder of the
two (v_max 1.7 m/s, 2,500 clamps a frame in its worst window). The
handled chain above (5.2 -> 4.1, 2026-09-01 evening) was hot and
covered cuts, list and filter only. A same-session moving window for
the head was not captured: both 30 s runs stilled, and the phone
locked before a third run.

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
cost that comes straight out of the particle budget: ~1.2 ms of a
moving frame and nothing at rest by the draw attribution above, the
131,072-point tracer draw ~0.5 ms of it and the liquid-glass fill
hidden behind the following compute. The fine split's 2.3 to 3.1 ms
surface span, and its 1.5 to 2.6 ms for the tracer draw, were
overlap.

### Next, in order

Amended 2026-09-03 by "The 4x session" below: item 2 shipped as the
cell-ordered layout; item 3's refine cut is dead at 4x too and the
LANES and workgroup A/B waits on an alternating instrument; item 1
belongs to the glass look, which Jack deprioritised at 4x.

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

## The 4x session (2026-09-02, evening)

Jack's question, verbatim: "is it possible to enable this app to run
on my iPhone *well* - *comfortably* at the 4x resolution setting?"
The session priced it: the phone under a throwaway instrument for the
cost, the laptop film harness for the physics, three read-only or
film-only agents for the levers. Nothing here changed the code; the
section after this one records what shipped.

### The instrument, and a protocol rule it forced

The instrument is a scratchpad patch on the head (`p4x/patch4x.py`,
never in the repository): `FLUID_IDLE=0` turns the gate off,
`FLUID_NMIN` pins the substep count, `FLUID_REFINE` pins the refine
pass count, and a cadence line prints the mean frame interval of every
120-frame window. The meter is the cadence line of a window that is
over budget; a window that fits locks at 8,334 us and says nothing,
and the stats line's GPU span is the governor's clock whenever the
frame fits (the M5 record).

**Alternate inside one run; never pair installs.** The same build
installed twice at eight pinned substeps read 12.2 then 11.3 ms a
frame, and two one-constant builds swapped order between the halves
of a mirrored chain (LANES 4: 12.0 then 9.9; workgroup 128: 9.8 then
11.4). The thermal state climbs from nominal to serious within two
minutes of over-budget frames, and every cost rises 25 to 35% with
it. So `FLUID_NMIN=6,8` and `FLUID_REFINE=5,2` take comma lists and
switch every 120 frames, and a pair read inside one run cancels the
drift. Two caveats of the instrument: the cadence line stamps each
window with the *next* window's refine pin (the substep stamp is
right), and the stats line's GPU p50 spans two windows.

The 2026-09-03 instrument moved the cadence line after the substep
selection, which shifted the substep stamp too: there the window's
own count is the one stamped a window later. The two builds therefore
read their stamps differently, and a run is mined against the pin list
it was launched with, never against the stamp alone. The check that
catches a wrong mapping: more substeps must cost more.

### The frame at 4x (reference device, 2026-09-02, glass unless named)

Frame = n x S + R: n substeps from the CFL (n = ceil(3.36 x v_max) at
120 Hz and the 4x spacing), S the cost of one substep, R the look's
own work each frame.

| Quantity | Cool (nominal) | Hot (serious) | Method |
|---|---|---|---|
| S, five refine passes | 0.97 to 1.0 ms | 1.25 to 1.36 ms | alternating pairs 6/8, 7/8, 5/7; four-point fits |
| one refine pass | ~0.1 ms a substep | ~0.1 | n = 8, five against two passes: 11.7 against 9.2 ms |
| S, two passes | ~0.65 (inferred: the cool S less a hot pass cost) | ~1.0 | |
| R glass | — | 2.4 ms | four-point fit, intercept |
| R flat surface | — | 1.6 | |
| R particle view | — | 1.15 | |
| R direction wheel | — | flat + ~0.7 | one pair, inside the drift |

The four-point fits (one run each, n = 6, 7, 8, 9 alternating, five
passes, phone hot): glass 10.5 / 11.9 / 13.2 / 14.6 ms; flat 9.1 /
10.3 / 11.6 / 12.9; particles 8.9 / 10.0 / 11.4 / 12.8. Each is a
line to within the window scatter.

The pinned ladder, cool, gate off, phone flat and still: two, three,
four and five substeps lock 8,334 us (five at GPU p50 7.8 ms; four
still locks at thermal serious); six reads 8,576 (a frame dropped
every 35); seven reads 10.3 to 10.4 ms; eight at five passes 10.4
cool and 11.6 to 13.2 hot; eight at two passes 9.2 hot. Eight on the
natural schedule is bistable at 8,334 or 10,400: when the frame fits,
the substep is 1.04 ms and gets two passes; when it slips, the
substep passes 1.05 ms and gets five.

Every look sleeps lying flat at 4x (gate on, 100 s a look): glass,
flat, particles and the wheel all settle at GPU p50 5.5 to 6.8 ms, the
governor's clock, and the idle counter climbs. Lying flat is the pose
in which gravity leaves the slab; see the boil below.

### The mechanism of the dip

At 120 Hz, six or seven substeps at five passes cost 8.6 to 10.4 ms
and do not fit. The frame slips. Two rules then read the measured
frame and feed each other: the CFL reads the measured dt and asks for
more substeps, and the refine rung reads the measured substep (1.47
ms at n = 7 and a 10.3 ms frame, above the 1.05 ms boundary) and
stays at five passes. Clamped water reads back exactly the clamp
speed, and `ceil` splits that tie, so the count climbs one step a
frame to the cap: 16 x 1.0 to 1.3 + 2.4 = 18 to 23 ms, 45 to 55 Hz,
until v_max falls under about 1.5 m/s. That is the M5 record's "dips
to 40 to 60 Hz for 3 s". The two-pass rung is reached only at exactly
120 Hz with eight or more substeps, which the slip prevents.

### What the films found (Jack's laptop, 2026-09-02, SPACING=0.0062)

1. **The 4x pool boils at rest, upright.** At natural pacing (two
   substeps of 4.17 ms) a 15 s upright hold reads v_max 0.60 to 0.75
   m/s every second, 1,000 to 1,700 CFL clamps a second, compression
   max 0.16 to 0.19%, flicker 162,016 and 162,114 px/frame against
   the 12,000 line — every tracer dot in the body moving — and the
   WAKE film never sleeps (v_max 0.05 against V_SLEEP 0.04 at 20 s).
   Pinned at four substeps (2.08 ms) it rests: v_max 0.08, zero
   clamps, compression 0.03%, flicker 10,491 and 10,710, sleep at
   frame 1652. At three substeps (2.78 ms) five passes read 14,599
   and 15,464; eight passes 9,396 and 10,751. Eight passes at 4.17 ms
   also rest it (v 0.12, flicker 10,613 and 10,793, sleep at frame
   2084) with 2,800 clamps in the settled 3 s and compression 0.08%:
   cheaper a frame (40 sweeps against 56) and dirtier. The flat pose
   settles at every setting, which is why the desk never showed it.
   Jack confirmed the boil on the phone the same evening: "it
   absolutely does jitter/boil at 4x". The M3 convergence ladder
   ("refine depth changes nothing at either length") was 1,620
   particles; at 6,468 the 4.2 ms substep is a convergence failure
   as well as a timestep one.
2. **The refine mid rung is dead at 4x as at 1x.** At 2.08 ms
   substeps five passes are required (four: 11,048 and 11,271, under
   the line but outside the five-pass pair's spread; three: 41,910
   and 41,968; two: 158,869 and 162,989). At 1.39 ms four passes hold
   (9,482 and 9,733 against 9,275 and 9,798; 14% fewer sweeps) and
   three fail narrowly (12,397 and 12,570). The short rung at 1.04 ms:
   two passes read 26,389 at rest and 0.068% under the shake; three
   passes 11,196 and 0.049%; four 9,805. Ring meters do not
   discriminate, and shake clamps span 9,713 to 15,742 across
   identical rows.
3. **Spray does not set the substep count.** An interior-only CFL
   (particles above half rest density) buys 1.8 to 5.6% fewer
   substeps over the shake film, inside the 2.5% run-to-run scatter,
   and nothing in ring, tilt or spin; the feared threefold feedback
   (a spray particle at three times the clamp read back into the next
   frame's CFL) never appears: the largest frame-to-frame jump in n
   is 2 to 3. The interior water itself reaches the cap's own clamp
   (4.76 m/s) in the film's shake. The mechanism that is real is the
   clamp-tie ratchet above.
4. **The sort's gate.** The design for a cell-ordered layout (below)
   asked for one device experiment before any build: seed the
   lattice in row order (the shipped seed), shuffled (what handling
   produces) and cell order (what a sort delivers), and read eight
   pinned substeps at five passes, 60 s a run, mirrored. Row 9.8
   (cool), shuffled 11.1, cell 9.8, cell 10.6, shuffled 13.1, row
   11.5 (hot). Shuffled sits 1.3 to 2.5 ms a frame above its
   neighbours in both halves, above the ~1 ms drift: 0.16 to 0.3 ms a
   substep, 16 to 30%. Cell equals row inside the drift. So a sort
   recovers what handling mixes and nothing at rest. Unexplained:
   both shuffled runs read a livelier pool (v_max 0.3 against 0.18 or
   less); the cost gap is not obviously from it.
5. **LANES 4 and workgroup 128 are unmeasured, not dead.** Both were
   built and installed; paired installs drift as above, so neither
   number means anything. A constant baked into a shader needs both
   pipelines in one build to alternate; that instrument is not
   written.

### The levers, priced

| Lever | Buys | Costs | Standing |
|---|---|---|---|
| The two-pass jump (a 120 Hz frame the CFL would put on six or seven substeps runs eight) | fits cool: 8 x 0.65 + 2.4 = 7.6 ms; at 3 m/s cool converges near 87 Hz instead of the cap | nothing at rest; hot it changes nothing by construction | shipped, below |
| The cap scaled with the spacing (2.6 ms at 4x, four substeps at rest) | the rest boil; sleep | rest frame 4 x 1.0 + R until the gate sleeps (~14 s) | shipped, below |
| The cell-ordered layout | 0.16 to 0.3 ms a substep in mixed water; hot particle view with the jump: 8 x 0.75 + 1.15 = 7.2 | 6 to 10 h; five buffers, four bindings | shipped, below |
| Tracers halved at 4x | 0.4 to 0.8 of glass's 2.4 | a look change | Jack's dial; glass deprioritised 2026-09-02 |
| LANES / workgroup retune | +-10% | an alternating two-pipeline instrument | deferred |
| Pre-clamp speed for the CFL | n answers in one frame both ways | none | quality, not cost; open |
| Interior-only CFL | 2 to 5% | clamps +2 to 47% in motion | dead |
| Refine mid rung | 14% of sweeps at 1.39 ms with four passes | flicker | dead |

What "comfortable" can mean, priced: 120 Hz through ordinary and
brisk handling, with a dip only in a hard shake and brief — reachable,
and on the flat and particle looks reachable hot; glass at 4x is
comfortable cool and dips once the phone is hot. Never dipping, any
shake, needs a substep under 0.4 ms, 2.5 to 3x off, and is not on
the table. Jack's ruling, 2026-09-02: glass deprioritised ("flat and
particle look the coolest"), the first definition accepted, no more
phone time that day, the work approved.

### What shipped (branch m5-4x, 2026-09-03)

Jack ruled the evening before: the boil is real on the phone, the
glass look is deprioritised at 4x, the first definition of comfortable
is the target, no phone time that day, the work approved. Two agents
built the three changes in parallel worktrees; a fresh-context review
followed (below). Every number in this section is Jack's laptop
(Apple M2 Max), 2026-09-03; the phone has not run this build.

1. **The cap scales with the spacing** (`SUBSTEP_PER_SPACING`, 0.42 s
   per metre: 4.2 ms at 0.01 m, 2.6 ms at 0.0062 m, so a 120 Hz frame
   at 4x floors at four substeps). Each `Sim` carries its own cap and
   `substep_floor` takes it. The refine rung is unchanged and named:
   `REFINE_SHORT_DT`, 1.05 ms. Films at 4x, natural pacing, before and
   after: meterup flicker 161,469 and 148,401 -> 11,104 and 10,932
   px/frame (line 12,000); the wake film slept at frame 1658 where the
   before never slept; shake compression max 0.198 and 0.168 -> 0.084,
   0.082 and 0.074%; spin 0.050 -> 0.038%; flat 0.036 -> 0.014%. At
   1x nothing moves: the cap is the same length and the rest histogram
   is unchanged (meterup 9,789 and 9,792 -> 9,259 and 10,329).
2. **The two-pass jump** (`substeps_for`, one pure function that the
   production frame and the film harness both call): the CFL count
   floored by `substep_floor`; then, iff eight substeps of this frame
   land on the two-pass rung (`refine_passes(dt / 8) == 2`, the same
   division the encoder makes) and the count is six or seven (above
   `CHEAP_RUNG_COST` 0.65 times eight), eight; then the cap. A
   slipped frame keeps its count. Over the 4x shake film the 6 and 7
   bins (30 and 23 frames) emptied into the 8 bin; the jump adds 0.01
   to 0.02 points of compression max against a no-jump binary (0.071
   and 0.064%), the two-pass rung's residual, accepted. Four pure
   tests pin the mapping 5, 6, 7, 8, 9 -> 5, 8, 8, 8, 9 at 1/120 s, a
   10 ms frame keeping 7, a flung frame at the cap, and the floor at
   both spacings.
3. **The cell-ordered layout.** `scatter` copies the five persistent
   records (positions, velocities with the acceleration mean in w,
   prev_vel with the pressure mean in w, prev_pressure, temperature)
   from the resting set into a working set at each particle's
   cell-ordered slot; the substep's sweeps bind the working set;
   `density_div` walks `starts[c]..starts[c+1]` with j = k and stores
   k in the neighbour list; `integrate` writes the resting set back at
   the same slot; `reduce_stats` and every per-frame reader bind the
   resting set, which is canonical. The sorted index list is retired.
   The solve layout binds 23 storage buffers (`SOLVE_STORAGE_BUFFERS`
   sets the device limit at both creation sites); the binding count
   has run on the laptop only. The microbench owns its own count and
   scatter now (about 25 lines beside its old stencil kernel); its
   fate is still Jack's. Cost: 56 bytes in and out per particle per
   substep, +362 KB at 4x. Tests: the colours-still test follows a
   particle by position across the 27 surrounding cells with the 0.01
   walk bound unchanged (measured walk a frame: velocity 0.0054,
   acceleration 0.0008, pressure 0.0006, proximity 0.0011, direction
   0.0002 — the M5 record's numbers); a new test checks the working
   set's cell order, the scan total and the bit-for-bit travel of all
   five records after a 60-frame settle; every sixteenth density is
   pinned to a CPU brute-force sum with the analytic wall fill within
   1e-3 (a wall-fill mutation every stats assertion passed failed it,
   814.57 against 796.52). Films, three a side: 1x meterup 9,854 /
   8,936 / 10,414 -> 9,865 / 10,080 / 9,977; 1x shake 0.138 / 0.088 /
   0.076 -> 0.057 / 0.119 / 0.067%; 4x at four substeps: meterup
   11,954 / 10,672 / 11,584 -> 11,072 / 11,704 / 11,253, shake 0.073 /
   0.070 / 0.061 -> 0.049 / 0.071 / 0.056%, wake sleep at 1814 / 2086 /
   1777 -> 1855 / 2087 / 1844. No film moved outside its scatter.

The integrated head, gate green, films on the same laptop the same
day: 4x natural pacing meterup 10,931 and 9,551 px/frame, shake
0.075 and 0.080%, wake sleep at frame 1855, spin 0.038%, flat 0.014%,
drag 0.045%; 1x meterup 9,000 and 10,056, shake 0.088 and 0.097%,
wake sleep at frame 1760. Every 4x guard that failed on the head
before this branch passes, and every 1x guard sits in its band.

Two bands in earlier sections did not reproduce on the laptop that
day and are scatter, not shifts: the 1x shake compression read 0.076
to 0.138% on the unchanged head (the record's 0.03 to 0.06 was one
session), and the 4x four-substep meterup read 10,672 to 11,954
against the 10,491 and 10,710 of the day before. Read every film band
as a distribution.

### The device session (2026-09-03, evening)

The runbook above, run. The build measured is this branch's head with
the review's fixes in it (below), except where a line names the build
before them. Every comparison against d7560a8, the branch point, is a
difference taken inside one run: two pinned substep counts alternate
every 120 frames, refine is pinned to five so the arms never change
rung, and the mean interval of an over-budget window is the meter. The
check that a mapping is right is that more substeps cost more.

**The binding count.** The app launches at 4x and runs: 6,468
particles, 4,840 cells, spacing 0.0062 m, no pipeline-layout error out
of `Renderer::new`. The 23-buffer solve layout is inside the A15's
limit. Lying flat the same build rests at four substeps and sleeps at
frame 1,879.

**The 4x boil, killed.** Jack held the phone upright, this branch
first and then d7560a8, minutes apart, one pose, one thermal state.

| Upright, held, 4x | d7560a8 | This branch |
|---|---|---|
| Substeps | 2 to 4, oscillating | 4, flat |
| v_max | 0.35 to 0.89, never falls | 0.07 to 0.10 |
| CFL clamps | 4,280 a second, without end | none after frame 600 |
| Compression, mean and worst | 0.17 to 1.20% / 15.6% | 0.15% / 2.5% |
| Frame interval p50 | 8,334 us | 8,334 us |

Both hold 120 Hz, so the cap costs no cadence at rest: it spends GPU
headroom that was there already (p50 6.4 against 6.9 ms).

**The sort pays twice.** Eight pinned substeps, five passes, 4x,
mirrored runs, the pool seeded in row order and in shuffled order.
GPU p50, which reads the work here because the frame is over budget
and the count does not alternate:

| Seed order | This branch | d7560a8 (2026-09-02) |
|---|---|---|
| row | 8,394 and 8,360 us | 9,701 and 11,460 |
| shuffled | 8,458 and 8,382 | 10,962 and 13,068 |

Shuffled costs 0.5% more than row order with the sort and 13 to 16%
more without it. That was the sort's whole justification and it holds.

The second payment is larger and was not predicted. A settled pool has
already drifted out of whatever order it was seeded in, so the sort is
not idle at rest:

| 4x, five passes, thermal serious | Substep | Frame at eight |
|---|---|---|
| d7560a8 | 1,235 us | 11,334 |
| This branch, before the review's fixes | 1,013 | 9,711 |
| This branch, after them | 955 | 9,236 |

Eighteen per cent a substep. The two branch rows differ by less than
the thermal column moved between them; the read-only binding and the
deleted field changed nothing measurable. Each fit's intercept is the
flat look's own per-frame work, and the two runs pinned at eight and
twelve agree with each other and with the 1.6 ms this record measured
for that look by a different method on a different day: 1,609 and
1,596 us. A third run pinned at eight and sixteen reads 1,150, so the
intercept carries a few hundred microseconds of its own uncertainty —
the frame is not quite linear in the count within a run, because the
phone heats as the run goes on. The agreement of the two is still the
best independent check in the session: one number confirms the meter,
the window mapping and the look's cost together.

At 1x the same measurement finds nothing, which is the answer that
mattered: 1x is the shipped default, and the permute is unconditional.
The frame fits at every count the app allows there, so neither meter
reads it — the interval locks at 8,334 us and the GPU span becomes the
governor's clock (6,403 us at eight substeps against 6,515 at
sixteen). `FLUID_SIM` raises the substep cap, and pinned counts of 32
and 48 put both arms over budget, where the cadence line is a meter
again: 222 us a substep on this branch against 226 on d7560a8, a 1.6%
difference inside the scatter. Sixteen hundred particles fit in cache,
so there is no locality to win and the permute is cheap; 6,468 do not,
and it is worth eighteen per cent.

**A brisk swirl at 4x, hot, on the shipped schedule.** Jack swirled
the phone without pause for 70 seconds a look, gate on, no pins, the
phone already hot from twenty minutes of over-budget runs.

| | Flat | Particles |
|---|---|---|
| Median second | 8,334 us | 8,334 us |
| Seconds over budget | 16 of 70 | 24 of 70 |
| Longest unbroken dip | 2 s | 4 s |
| Worst second | 9,787 us | 11,806 |
| v_max reached | 2.19 m/s | 3.06 |
| Substeps seen | 4 to 10 | 4 to 16 |

That is definition (a), measured: 120 Hz through brisk handling, and
the dips are seconds, not the sustained 45 to 55 Hz the dip used to
hold until v_max fell under 1.5 m/s. Jack's eye on the particle look,
2026-09-03: "it's stunning... enabling that to run at 4x without a
hitch in the 120fps". His eye on the flat look the same minute: the
flecks "just seem to teleport", which is a fault of the field path,
not of this branch — the particle view of the same water is clean.
Booked as HANDOFF O7 with the wash it shares a cause with.

**What the device said about the jump.** The instrument counts, per
window, the frames whose substep actually landed on the five-pass
rung. At eight pinned substeps at 4x it is 3 to 5 frames in 120: the
display's own clock holds a 120 Hz frame under 8.4 ms, so eight
substeps stay on the two-pass rung for 97% of frames. The laptop's
fixed 1/120 could not have shown this either way, and the jump's whole
premise is that the division lands there.

REVIEW.md's device-measurement blocker is answered for this branch.

### The review, and what it changed (2026-09-03)

Two fresh-context reviewers read the diff and the repository, never
the plan: one on the GPU work and its encoders, one on the rules and
the honesty of the code's claims. Every finding then went to a
reviewer told to refute it. Fourteen findings, eight surviving.

The GPU lens cleared the sort's correctness in detail and filed
nothing against it: the five records the split copies and integrate
writes back, the self-exclusion in `density_div` now that both indices
are working slots, every reader of the neighbour list's stored slot,
the per-substep scratch that needs no copy, `positions[].w` across the
copy, the per-frame draws and `reduce_stats` binding the resting set,
the three encoders still matching, and the bind layouts against the
raised limit.

What changed as a result:

- `prev_pressure` is declared `read`, not `read_write`. No solve
  kernel has written it since the resting copy took the write.
- The cap's comment dated 4.2 ms to 2026-08-31. This record says 2.2
  ms was set that day and 4.2 ms decided on 2026-09-01; the comment
  now says so, and carries back the sentence about clearing half a
  frame, which the constant still needs.
- `CHEAP_RUNG_COST`'s doc comment derived 0.69 to 0.78 from the
  numbers it cited, not 0.65. The constant is the cool two-pass
  substep against the cool five-pass one, 0.65 ms against about 1.0,
  and says that now. Both lenses found this independently.
- `dt_sub_max` is gone as a field and as a parameter. It was
  `SUBSTEP_PER_SPACING * spacing` and every caller already held the
  spacing, so the pair could only ever disagree with itself.
- One test added: at the 4x spacing a 120 Hz frame floors at four
  substeps and still jumps to eight once the CFL asks for six.

Refuted and not acted on: that the substep tests never reach 4x (the
floor of four cannot jump, so no shipped rung was unpinned — the new
test pins it anyway); that four records describe code that no longer
exists; that the resting and working comments contradict each other;
that the CFL inequality lost its only statement; that the film
harness's frame length duplicates `NOMINAL_FRAME`.

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
