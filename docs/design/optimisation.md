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

## The device runbook (waits for the phone)

1. Deploy the XSPH-6 build; Jack's eye on the swirl.
2. The basin A/B at the chosen cap: hard shake, watch for the 60 Hz
   lock, its exit, and GPU p50 on both sides.
3. Settled GPU p50 at the chosen cap (expect roughly half the rest
   solver cost at 2 substeps).
4. The M3 exit measurements: settled upright hydrostatics, the
   minute hand-test with intervals, the "inside budget" clause.
5. Budget O2 firmed; the battery bound measured.
