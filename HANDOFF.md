# Handoff — project state

*Updated 2026-08-30. Audience: the next agent, or Jack. This file is the
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
| M1 surface | Done 2026-08-30 on the reference device; measurements below. CI on the branch pending push. | `docs/design/m1-surface.md` |

Test baseline: 8 Rust tests pass, 2026-08-30.

## M1 measurements (reference device, 2026-08-30, Release)

~96 s sustained, captured over `devicectl` console. 120 Hz locked: frame
interval p50 = p99 = 8,334 µs, steady-state max 16,668 µs (a rare single
dropped frame), ~2 s startup transient. CPU encode+submit+present p50
~1.4 ms (includes drawable-acquire back-pressure). `phys_footprint`
63.7 MB. Battery 100%, thermal nominal throughout. The adapter offers no
`TIMESTAMP_QUERY`; GPU pass time reads 0 until a finer probe exists.
Idle verified: stats freeze when backgrounded, resume on foreground.

## The next task — M2, the particles

Jack's directive, 2026-08-30: something visual and satisfying on the
phone as soon as possible, then optimise heavily. That is M2: a GPU
particle buffer, integration under the body force, box collision, and
point rendering — tilt the phone and tens of thousands of particles
slosh. The first WGSL shaders and the `wgsl` feature enter here.

Start with the design record, `docs/design/m2-particles.md`. After the
visuals land, the heavy-optimisation pass: firm budget O2, run the
battery bound, run the frame-latency-1 experiment
(`docs/design/m1-surface.md` section 6).

## Open decisions

Binding on nothing. Decide each when its milestone needs it, then move it
to `docs/design/decisions.md`.

| Id | Question | Where it is decided |
|---|---|---|
| O1 | The simulation method. Position-based fluids (PBF) on the GPU is the proposal: cheapest path to a convincing slosh, field colouring is free. MLS-MPM is the fidelity upgrade. Jack has not confirmed either. | M3 design record |
| O2 | The performance budget at 120 Hz on the reference device. Provisional, from M1: frame interval p99 ≤ 8,400 µs sustained; CPU frame path p99 ≤ 2 ms; footprint ≤ 200 MB; thermal nominal over ten minutes. Firmed in the optimisation pass after M2. | Optimisation pass |
| O3 | The name. "Fluid Box" is a working title; the iOS target is `FluidApp`, bundle `com.screendead.FluidApp`. | Jack |
| O4 | Moot 2026-08-30: the web target is removed (D1 amendment). | — |
| O5 | The license. `Cargo.toml` says `UNLICENSED` until Jack chooses. | Jack |
| O6 | Rotation. `CMDeviceMotion.rotationRate` is not read. Coriolis and Euler forces on the fluid arrive when a milestone asks for them (M7). | M7 design record |

## Roadmap

Each milestone has an oracle. A milestone is done when its oracle passes on
the reference device and `HANDOFF.md` records the measurement.

| Milestone | What it builds | Oracle |
|---|---|---|
| M0 Toolchain | Body force from Rust on the phone | The readout agrees with CoreMotion; signs verified on the phone. **Done 2026-08-30.** |
| M1 Surface | wgpu owns a `CAMetalLayer` from `fluid-core`; clear and present at display rate; frame-time capture | 120 Hz stable, idle draw measured, budget O2 set. **Done 2026-08-30.** |
| M2 Particles | GPU particle buffer, integration under the body force, box collision, point rendering | Tilt and push the phone; particles behave; particle count at budget recorded |
| M3 Fluid | The method from O1: neighbour search on the GPU, incompressibility, viscosity | A convincing slosh inside budget; incompressibility measured |
| M4 Water | Screen-space fluid rendering: depth, smoothing, normals, refraction; the box itself | Looks like water; inside budget |
| M5 Views | Colour by density, pressure, velocity, acceleration; temperature as an added field | Each view switches with no frame drop |
| M6 Headroom | Adaptive substeps, sleep when still, thermal response; power measured | Battery draw recorded against a target |
| M7 Feel | Sensor-to-frame latency measured and tuned; haptics; rotation (O6) | Latency number recorded; Jack's hand says it feels right |

## Rules that bind future work

CLAUDE.md holds the rules. Three to hold in memory:

- Do not write scaffolding for a later milestone. Add a dependency when
  its first caller arrives.
- The comment rule and the performance rule are Jack's words. Quote them,
  do not paraphrase them.
- Every performance number carries the device and the date.

## Pointers

- `docs/design/decisions.md` — D1 stack, D2 shell, D3 frame and units,
  D4 dependencies; each amended 2026-08-30 for the web removal.
- `scripts/gate.sh` — the whole gate; CI runs the same steps.
- `platforms/ios/project.yml` — the Xcode project source; never edit the
  generated project.
