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
| Rust workspace, three crates, the gate, CI, VS Code config | Written 2026-08-30; gate not yet run | `Cargo.toml`, `crates/`, `scripts/gate.sh`, `.github/workflows/ci.yml` |
| iOS shell, the motion readout | Written 2026-08-30; not yet built | `platforms/ios/` |
| Web page, the motion readout | Written 2026-08-30; not yet built | `platforms/web/` |
| First commit | **Not made.** Jack has not yet said to commit. | — |
| M1 onward | Not started | — |

Test baseline: none measured yet. Performance baseline: none measured yet.

## In flight on 2026-08-30

Work that was running when this file was written. Check each before you
continue.

1. **Toolchain.** `rustup update stable` was running (from 1.83.0). When it
   is done: write `rust-toolchain.toml` with the exact version, then
   `cargo install cbindgen wasm-bindgen-cli@0.2.127`. The wasm-bindgen CLI
   must match the crate version in `Cargo.toml` exactly; 0.2.84 was
   installed before.
2. **Xcode.** `xcodebuild -downloadPlatform iOS` was running. Xcode 26.6 was
   installed without the iOS 26.5 platform, and a device build fails until
   it lands. Check with `xcodebuild -showsdks` and a device build.
3. **The gate has not run.** Expect `cargo fmt` to reflow the crate sources
   once; commit the reflowed form. Generate `crates/fluid-ffi/include/fluid_ffi.h`
   with the command in CLAUDE.md section 7 before the first gate run.
4. **Nothing has run on the phone.** The signing chain is proven up to the
   build step: a current "Apple Development: Jack Lusher" certificate
   (expires 2027-07-22), the Personal Team in Xcode, the phone paired with
   Developer Mode on.

## The next task — close M0

M0 is closed when the body force computed in Rust is on the phone screen
and in a browser, and the signs are right. Steps:

1. Finish the two in-flight installs above and run `scripts/gate.sh` green.
2. `scripts/run-ios.sh`. The readout shows CoreMotion's gravity and user
   acceleration and the body force from `fluid_body_force`. Face up at rest,
   body force reads about (0, 0, -9.81). Push the phone right: the x body
   force goes negative.
3. `scripts/build-web.sh && scripts/serve-web.sh`, then open the page in
   Chrome and drive the DevTools sensor panel. Face up at rest reads
   (0, 0, -9.81) too.
4. **Verify the web sign convention on the phone, not in emulation.** The
   conversion in `crates/fluid-web/src/lib.rs` rests on the W3C spec, and
   iOS Safari has had a sign quirk on `DeviceMotionEvent.acceleration`.
   Serve over TLS on the LAN (see O4), open the page on the reference
   device, and compare each row against the native app. If a sign differs,
   the fix is in `sample_from_device_motion` with a test, not in the page.
5. Record the observed numbers here and mark M0 done in the table above.

## Open decisions

Binding on nothing. Decide each when its milestone needs it, then move it
to `docs/design/decisions.md`.

| Id | Question | Where it is decided |
|---|---|---|
| O1 | The simulation method. Position-based fluids (PBF) on the GPU is the proposal: cheapest path to a convincing slosh, field colouring is free. MLS-MPM is the fidelity upgrade. Jack has not confirmed either. | M3 design record |
| O2 | The performance budget: frame time, GPU time and power at 120 Hz on the reference device. Set from M1 measurements, not from guesses. | M1 design record |
| O3 | The name. "Fluid Box" is a working title; the iOS target is `FluidApp`, bundle `com.screendead.FluidApp`. | Jack |
| O4 | TLS on the LAN for phone web testing. `DeviceMotion` needs a secure origin; plain `http://` from the Mac's IP is refused. `mkcert` plus a small TLS server is the likely answer. | M0 close |
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
