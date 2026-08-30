# Fluid Box

*Working name.*

**A box of liquid you hold.** Tilt the phone and the water runs to the low
corner; push it and the water sloshes back. The simulation runs on the GPU at
the display rate, driven by the motion sensors, and renders as water, as
particles, or coloured by density, pressure, temperature, velocity or
acceleration. One Rust source builds the iOS app and the website.

## The stack

Rust for everything that is not the platform's job. wgpu for the GPU, so one
WGSL shader runs on Metal on the phone and on WebGPU in the browser. A thin
Swift shell on iOS owns the sensors and the drawing surface and calls the
Rust core over a C ABI. A thin JavaScript page does the same on the web
through wasm-bindgen. Xcode is a toolchain, not an editor: everything builds
from the terminal or VS Code.

The design is in [`docs/design/decisions.md`](docs/design/decisions.md).
The rules are in [`CLAUDE.md`](CLAUDE.md). Where the project is right now,
and what comes next, is in [`HANDOFF.md`](HANDOFF.md).

## Running it

You need the pinned Rust toolchain (`rust-toolchain.toml`), Xcode with the
iOS platform installed, and `brew install xcodegen`, plus
`cargo install cbindgen wasm-bindgen-cli`. Then, from the repository root:

```sh
scripts/gate.sh        # format, lint, test, and cross-compile for wasm and iOS
scripts/run-ios.sh     # build, sign, install and launch on the phone
scripts/build-web.sh && scripts/serve-web.sh   # then open http://localhost:8080
```

The app needs a physical iPhone; the simulator has no motion sensors. The
web page needs a browser with WebGPU and, on a phone, a secure origin.

## Map

| Path | What it holds |
|---|---|
| `crates/fluid-core/` | The simulation and rendering core: pure Rust, no platform types |
| `crates/fluid-ffi/` | The C ABI for iOS, as a static library |
| `crates/fluid-web/` | The wasm-bindgen surface for the web |
| `platforms/ios/` | The Swift shell, generated into an Xcode project by XcodeGen |
| `platforms/web/` | The page and its glue |
| `scripts/` | The gate and the build and run scripts |
| `docs/design/` | Decision records |

## Ground rules

- Real-time performance and efficiency on the reference phone is the oracle.
  Every choice answers to it, and every performance claim carries a
  measurement.
- Code that no test asserts on and no run reaches does not enter the
  repository.
- The core has no platform code; shaders are WGSL only; the shells do only
  what the platform alone can do.
