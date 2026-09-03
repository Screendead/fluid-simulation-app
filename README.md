# Fluid Box

*Working name.*

**A box of liquid you hold.** Tilt the phone and the water runs to the low
corner; push it and the water sloshes back. The simulation runs on the GPU at
the display rate, driven by the motion sensors, and renders as water, as
particles, or coloured by how fast it moves, how hard it is thrown
about or squeezed, how crowded each drop is, or which way it is going —
that last one as a hue around the colour wheel. The source is Rust; the product is an iOS app.

## The stack

Rust for everything that is not the platform's job. wgpu for the GPU, with
WGSL shaders. A thin Swift shell owns the sensors and the drawing surface
and calls the Rust core over a C ABI. Xcode is a toolchain, not an editor:
everything builds from the terminal or VS Code.

The design is in [`docs/design/decisions.md`](docs/design/decisions.md).
The rules are in [`CLAUDE.md`](CLAUDE.md). Where the project is right now,
and what comes next, is in [`HANDOFF.md`](HANDOFF.md).

## Running it

You need the pinned Rust toolchain (`rust-toolchain.toml`), Xcode with the
iOS platform installed, `brew install xcodegen`, and
`cargo install cbindgen`. Then, from the repository root:

```sh
scripts/gate.sh        # format, lint, test, header drift, iOS cross-compile
scripts/run-ios.sh     # build, sign, install and launch on the phone
scripts/run-sim.sh     # launch in the simulator: a link-and-launch check
```

The app needs a physical iPhone; the simulator has no motion sensors.

## Map

| Path | What it holds |
|---|---|
| `crates/fluid-core/` | The simulation and rendering core: pure Rust, no platform types |
| `crates/fluid-ffi/` | The C ABI for iOS, as a static library |
| `platforms/ios/` | The Swift shell, generated into an Xcode project by XcodeGen |
| `scripts/` | The gate and the build and run scripts |
| `docs/design/` | Decision records |

## Ground rules

- Real-time performance and efficiency on the reference phone is the oracle.
  Every choice answers to it, and every performance claim carries a
  measurement.
- Code that no test asserts on and no run reaches does not enter the
  repository.
- The core has no platform code; shaders are WGSL only; the shell does only
  what the platform alone can do.
