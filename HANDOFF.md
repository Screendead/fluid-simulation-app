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
| Stack, shells, frame and units (D1 to D3) | Decided 2026-08-30 | `docs/design/decisions.md` |
| Toolchain: Rust 1.98.0 pinned, all three cross-targets, cbindgen 0.29.4, wasm-bindgen-cli 0.2.127, Xcode 26.6 with the iOS 26.5 platform | Done 2026-08-30 | `rust-toolchain.toml` |
| The gate | Green 2026-08-30 (4 tests) | `scripts/gate.sh` |
| iOS shell on the phone | Done 2026-08-30. Jack confirmed the readout: at rest, body force reads about (0, 0, -9.81); moving the phone right sends user acceleration x positive, body force x negative. | `platforms/ios/`, `scripts/run-ios.sh` |
| iOS shell in the simulator | Launched headless 2026-08-30 (iPhone 15 Plus, iOS 26.5, "FluidSim") | `scripts/run-sim.sh` |
| Web page | Wasm built; verified in desktop Chrome with synthetic `devicemotion` events: face-up gives (0, 0, -9.81), push right gives x negative | `platforms/web/`, `scripts/build-web.sh` |
| CI | Green on master since 2026-08-30 (run 33332535892) | `.github/workflows/ci.yml` |
| M0 | **Done 2026-08-30**, with one check deferred: see the web track note below | — |
| M1 onward | Not started | — |

Test baseline: 4 Rust tests pass, measured 2026-08-30. Performance
baseline: none measured yet; O2 sets it at M1.

## The next task — M1, the surface

M0 closed 2026-08-30: the gate is green, the app runs on the phone (Jack
confirmed the readout) and in the simulator, the web page is verified with
synthetic events in desktop Chrome, and CI is green.

One M0 check is deferred, not dropped. **The web sign convention is
unverified on a real iPhone.** The conversion in
`crates/fluid-web/src/lib.rs` rests on the W3C spec, and iOS Safari has had
a sign quirk on `DeviceMotionEvent.acceleration`. Sensor access needs an
https origin, Jack ruled out a local certificate authority, so the check
waits for the web track's real hosting. Until it runs, treat the web
conversion's signs as provisional: when the first hosted page is up, open
it on the reference device, compare each row against the native app, and
fix any difference in `sample_from_device_motion` with a test.

M1 starts with its design record, `docs/design/m1-surface.md`: wgpu enters
(D1 names it), `fluid-core` owns a `CAMetalLayer` on iOS and a canvas on
the web, clear and present at display rate, frame-time and GPU-time
capture, the first power measurement, and budget O2.

## Open decisions

Binding on nothing. Decide each when its milestone needs it, then move it
to `docs/design/decisions.md`.

| Id | Question | Where it is decided |
|---|---|---|
| O1 | The simulation method. Position-based fluids (PBF) on the GPU is the proposal: cheapest path to a convincing slosh, field colouring is free. MLS-MPM is the fidelity upgrade. Jack has not confirmed either. | M3 design record |
| O2 | The performance budget: frame time, GPU time and power at 120 Hz on the reference device. Set from M1 measurements, not from guesses. | M1 design record |
| O3 | The name. "Fluid Box" is a working title; the iOS target is `FluidApp`, bundle `com.screendead.FluidApp`. | Jack |
| O4 | Resolved 2026-08-30: no local TLS, no certificate authority on Jack's devices. The on-phone web check waits for the web track's real hosting. Jack's sensor rule (CLAUDE.md section 7) stands regardless: sensor data never crosses devices. | — |
| O5 | The license. `Cargo.toml` says `UNLICENSED` until Jack chooses. | Jack |
| O6 | Rotation. `CMDeviceMotion.rotationRate` is not read. Coriolis and Euler forces on the fluid arrive when a milestone asks for them (M7). | M7 design record |

## Roadmap

Each milestone has an oracle. A milestone is done when its oracle passes on
the reference device and `HANDOFF.md` records the measurement.

| Milestone | What it builds | Oracle |
|---|---|---|
| M0 Toolchain | Body force from Rust on the phone and in a browser | The readouts agree with CoreMotion; signs verified on the phone |
| M1 Surface | wgpu owns a `CAMetalLayer` on iOS and a canvas on the web, from `fluid-core`; clear and present at display rate; frame-time and GPU-time capture; the first power measurement | 120 Hz stable, idle draw measured, budget O2 set |
| M2 Particles | GPU particle buffer, integration under the body force, box collision, point rendering | Tilt and push the phone; particles behave; particle count at budget recorded |
| M3 Fluid | The method from O1: neighbour search on the GPU, incompressibility, viscosity | A convincing slosh inside budget; incompressibility measured |
| M4 Water | Screen-space fluid rendering: depth, smoothing, normals, refraction; the box itself | Looks like water; inside budget |
| M5 Views | Colour by density, pressure, velocity, acceleration; temperature as an added field | Each view switches with no frame drop |
| M6 Headroom | Adaptive substeps, sleep when still, thermal response; power measured | Battery draw recorded against a target |
| M7 Feel | Sensor-to-frame latency measured and tuned; haptics; rotation (O6) | Latency number recorded; Jack's hand says it feels right |

The web track follows each milestone: the same core, the page does
permissions and the canvas, and a page with no WebGPU says so plainly.

## Rules that bind future work

CLAUDE.md holds the rules. Three to hold in memory:

- Do not write scaffolding for a later milestone. Add a dependency when its
  first caller arrives. `wgpu` arrives at M1.
- The comment rule and the performance rule are Jack's words. Quote them,
  do not paraphrase them.
- Every performance number carries the device and the date.

## Pointers

- `docs/design/decisions.md` — D1 stack, D2 shells, D3 frame and units.
- `scripts/gate.sh` — the whole gate; CI runs the same steps.
- `platforms/ios/project.yml` — the Xcode project source; never edit the
  generated project.
