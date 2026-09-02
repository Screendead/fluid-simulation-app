# Decisions

Each record binds the code. Amend a record by an explicit edit that keeps the
original and states the change.

## D1 — Stack: Rust, wgpu, WGSL (2026-08-30)

**Decision.** The simulation and the rendering are Rust on wgpu, with WGSL
shaders. One source builds the iOS app and the website.

**Why.** Two requirements fix the choice: one source for the phone and the
web, and a GPU-compute fluid simulation. On the web that forces WebGPU. The
single-codebase routes to WebGPU on the web and Metal on iOS are wgpu (Rust)
and Dawn (C++). wgpu's dual-target tooling is the cleaner of the two, and
one WGSL file runs unchanged on both. WebGPU is on by default in Safari
from iOS 26; the reference device has it.

*Amended 2026-08-30, Jack's call.* The web target is removed entirely,
mid-M1: the product is the iOS app alone. The stack stands — wgpu remains
the GPU layer and WGSL its shader language, now for their own qualities
rather than for a second platform. The original web motivation below no
longer binds; `crates/fluid-web` and `platforms/web` live only in git
history.

**Rejected.** Unity: a heavy runtime, web compute still experimental, poor
battery. Flutter: no compute shader access. Godot 4: WebGL 2 on the web, no
compute. Kotlin Multiplatform: no GPU story. TypeScript plus WebGPU in a
WKWebView shell: the GPU work is the same, but sensor rate, frame pacing and
battery sit behind a webview ceiling the project cannot remove later.

## D2 — Shells: a Swift shell on iOS, wasm-bindgen on the web (2026-08-30)

**Decision.** The iOS app is a Swift shell that owns a `CAMetalLayer`,
CoreMotion at 100 Hz or more, permissions, haptics and the app lifecycle,
and calls `fluid-core` through the C ABI in `fluid-ffi`. XcodeGen builds
the Xcode project from `project.yml`; the generated project is not
committed. The web build is `fluid-web` through wasm-bindgen with a plain
JavaScript page that owns permissions and the canvas.

The deployment target is iOS 17.0. The reference device is CLAUDE.md
section 5.

*Amended 2026-09-02 (D6).* The shell also owns the settings menu, the
readout, and their persistence.

*Amended 2026-08-30, Jack's call.* The original text read "There is no
simulator target: the simulator has no motion sensors." The project now
builds for the simulator (`aarch64-apple-ios-sim`) as a compile, link and
launch check, run by `scripts/run-sim.sh` and usable when the phone is
away. The simulator still has no motion sensors and proves nothing about
behaviour or performance; every measurement is on the reference device.

*Amended 2026-08-30, Jack's call.* The web shell is removed with the web
target (D1 amendment). The Swift shell paragraph alone binds.

**Why.** "Feels like holding a box of liquid" is a latency and sample-rate
requirement. CoreMotion's sensor fusion, delivered straight into the core
over FFI, gives the rate and the jitter the requirement needs. A webview or
a bridge caps DeviceMotion at 60 Hz with worse jitter. The shell also gives
ProMotion frame pacing, Core Haptics, and battery control.

**Rejected.** winit on iOS: it still needs CoreMotion through Objective-C
bindings, and its iOS support is thinner than a 200-line Swift shell. A
pure-Rust shell through `objc2` frameworks: possible, and fights the
platform to save a small file.

## D3 — Frame and units (2026-08-30)

**Decision.** The box is fixed to the device, so the device frame is the
box frame: x to the right of the screen, y to its top, z out of it. Sensor
input enters the core as `MotionSample`, whose `gravity` and
`user_acceleration` are in g with the CoreMotion sign convention: a phone
face up at rest reads gravity (0, 0, -1). The core works in SI. The body
force per unit mass on the fluid is `g · (gravity - user_acceleration)`,
which is the negated proper acceleration of the device.

The web page's `DeviceMotionEvent` vectors are the reaction to gravity, the
opposite sign. `fluid-web` converts them to a `MotionSample`; the page does
no arithmetic.

**Why.** CoreMotion supplies the fused split of gravity from user motion,
and later milestones want gravity alone (the box's "up" for rendering, and
haptics). The body force needs only the difference, and the identity with
the negated accelerometer reading is a test.

*Amended 2026-08-30, Jack's call.* The web conversion paragraph above and
its pending sign check are moot: the web target is removed (D1
amendment). Sensor input is CoreMotion alone.

## D4 — M1 dependencies (2026-08-30)

**Decision.** M1 adds one workspace dependency: wgpu 30.0.1, default
features off, features `std`, `metal`, `parking_lot`.

*Amended 2026-08-30, Jack's call.* The wasm feature set and
`wasm-bindgen-futures` left with the web target (D1 amendment). The
paragraph above is the surviving decision; the rejections below stand.

**Rejected.** `pollster`: not needed; wgpu 30's native adapter and device
futures are ready on first poll (verified in source), and `fluid-ffi`
resolves them with a noop waker. `raw-window-handle` and `web-sys` as
direct dependencies: wgpu re-exports both (`wgpu::rwh`, `wgpu::web_sys`),
and the `CAMetalLayer` path (`SurfaceTargetUnsafe::CoreAnimationLayer`)
needs neither. The `wgsl` feature: M1 has no shader; naga's WGSL frontend
enters at M2 with the first shader.


## D5 — M3 method: DFSPH in a 3D thin slab (2026-08-30)

**Decision.** M3 simulates incompressible Navier–Stokes with DFSPH —
divergence-free SPH: a neighbour grid, an iterative divergence-free
solve, an iterative constant-density solve, Morris viscosity, real SI
constants throughout. The domain is 3D: the screen at physical size by
the device's 7.65 mm depth. Driven by Jack's directive of 2026-08-30
(verbatim in the M3 record): maximum physical accuracy at the device's
real size.

**Why DFSPH.** Its pressure and density are physical fields in pascals
and kg/m³ — Jack's named lenses fall out of the state. It holds density
error to a stated target at real-time cost, and it keeps particles,
which the M4 water renderer consumes directly. (Amended 2026-08-31:
M4 consumes the particles through the splatted thickness field, not
through a per-particle depth pass; the M4 record holds the pipeline.)

**Rejected.**

- PBF: pressure is a constraint multiplier, not pascals; fails the
  accuracy directive even where the motion convinces.
- WCSPH: the real speed of sound, 1482 m/s, forces nanosecond timesteps;
  the usual artificial speed of sound fakes the equation of state, which
  the directive forbids.
- MLS-MPM: grid transfer dissipates the lively slosh, and its scatter
  atomics are hostile on a mobile GPU. Recorded fallback if DFSPH misses
  the budget.
- A pure Eulerian pressure grid (Jack's question, 2026-08-30): no
  particles means level-set surface tracking, notorious mass loss at a
  free surface, and nothing for M4 to splat.
- 2D in the screen plane: discards z physics in a slab real water moves
  through in 3D, and M4 needs depth. If the stage-0 envelope kills 3D,
  amend here with the numbers.

**Dependencies.** None new: the prefix scan and reductions are
hand-written WGSL in `fluid-core`.

*Amended 2026-08-30, from the design review and stage 0.* The budget
envelope resolves the slab depth at two particle layers at the chosen
spacing; "3D" stands as quasi-3D — z motion and M4 depth exist, z
eddies do not. 2D remains rejected. Wall boundaries are analytic planar
kernel integrals, not boundary particles: six flat walls have a closed
form. This decision closes O1.

## D6 — The menu is the shell's; the fluid is the core's (2026-09-02)

**Decision.** The controls Jack asked for on 2026-09-02 (M5 record)
are SwiftUI in the shell: the tap, the button, the half-sheet menu,
the readout, and the choices' persistence in `UserDefaults`. The core
exposes three calls for them — `set_particles(scale)`,
`particles_at(scale)`, `set_look(look)` — and computes everything
about the fluid from them: the spacing behind a scale, the count it
seeds, the flat colour's shading. The shell computes nothing about
the fluid; the performance rating beside each scale is a table of
device measurements, not a computation.

*Amends D2.* D2's shell list gains the settings menu, the readout,
and their persistence.

**Why.** Native controls, the system colour picker, the thermal state
and `UserDefaults` are platform work; a menu drawn in wgpu would put
a UI toolkit inside the platform-free core for no gain. The readout
rides the once-a-second stats call the console line already makes,
so it costs the frame nothing new.

**Rejected.** A UI toolkit in the core (egui, or a text pass over
wgpu): a dependency, a second render path, and no system colour
picker. Settings in the core: the core has no storage and should not
gain one. A rating computed live from the running frame: it can only
rate the scale that is running; the table rates the four before you
pick.
