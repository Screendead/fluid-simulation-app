# M1 — The surface

*Design record, 2026-08-30. Binds the code with `decisions.md`.*

## 1. Goal

wgpu enters. `fluid-core` owns a renderer. The renderer clears a surface and
presents it at display rate: a `CAMetalLayer` on iOS, a canvas on the web.
The clear colour comes from the body force, so a tilt of the phone shifts
the colour. That proves the path sensor → core → GPU → screen with no
shader and no simulation.

The oracle: 120 Hz stable on the reference device, idle draw measured,
budget O2 set from measurements.

## 2. Architecture

`fluid-core` gains one public type, `Renderer`. It owns the wgpu device,
queue, surface and configuration, and the timing capture.

- `Renderer::new(instance, surface, width, height)` — async. Requests the
  adapter and device, configures the surface. wgpu types cross this
  boundary; platform types do not. The shells make the surface:
  `fluid-ffi` from a `CAMetalLayer` pointer
  (`SurfaceTargetUnsafe::CoreAnimationLayer`), `fluid-web` from a canvas
  (`SurfaceTarget::Canvas`).
- `Renderer::frame(sample: MotionSample)` — encode one render pass that
  clears to the body-force colour, submit, present. No allocation.
- `Renderer::resize(width, height)` — reconfigure the surface.
- `Renderer::stats()` — percentiles from the timing rings. Called off the
  frame path, about once per second.

The shell owns the cadence; the core owns the frame. On iOS a
`CADisplayLink` at 120 Hz drives `fluid_renderer_frame` through the C ABI.
On the web, `requestAnimationFrame` drives `WebRenderer.frame`.

On native, wgpu's adapter and device futures are ready on first poll
(verified in the wgpu 30.0.1 source: both return `ready(..)`). `fluid-ffi`
resolves them with one poll of a noop waker. No executor dependency.

## 3. Surface configuration

| Setting | Value | Why |
|---|---|---|
| Format | The surface's preferred format | No conversion cost |
| Present mode | `Fifo` | Vsync; the only universally supported mode |
| Frame latency | Start at 2; measure 1 against it | Each step of latency is one drawable of memory (~13.7 MB at native resolution) |
| Alpha | `Opaque` | Nothing composites behind the box |
| Usage | `RENDER_ATTACHMENT` | A clear needs nothing else |

No depth buffer. No shader module, so the `wgsl` feature stays off; naga's
WGSL frontend enters at M2 with the first shader.

## 4. Timing capture

- CPU: wall time of encode+submit+present per frame, in a fixed ring of
  240 `f32` microseconds.
- GPU: `TIMESTAMP_QUERY` when the adapter offers it (Metal does), two
  timestamps around the pass, resolved into a buffer and read back through
  a small ring of staging buffers with `map_async`. Never a blocking wait
  on the frame path. On WebGPU the feature is requested only if offered;
  without it the page reports CPU time alone.
- Cadence: the shell measures it from `CADisplayLink` timestamps (iOS) or
  rAF timestamps (web) — the achieved rate, not the requested one.
- Memory: the iOS shell reads `phys_footprint` and prints it in the stats
  line.

The iOS shell prints one stats line per second; `devicectl` launch with
console attached captures it. The readout view shows the same numbers.

## 5. Idle

M1 idle means not visible. The iOS shell pauses the display link when the
scene leaves the active state. The page cancels its rAF loop when the
document is hidden. Either way no frame runs and the GPU does nothing;
the measurement is the stats counter freezing. Sleep-when-still is M6's
work; nothing here anticipates it (CLAUDE.md bans the scaffold).

## 6. Power

The first power measurement is a bound, not a precise number: battery
percentage over a timed sustained run at 120 Hz, with screen brightness
and Low Power Mode state recorded. Battery granularity is 1%, so the run
is at least 30 minutes. The proper power harness is M6's work.

## 7. Dependencies

D4 in `decisions.md` records the set. wgpu 30.0.1 (D1 names wgpu) with
per-target features, and `wasm-bindgen-futures` in `fluid-web` (lockstep
with wasm-bindgen, D2; wgpu's webgpu backend already pulls it). Nothing
else. pollster, raw-window-handle and a direct web-sys are all avoided;
section 2 and D4 say how.

## 8. Tested and exercised

Pure pieces — the body-force-to-colour map and the percentile math — get
unit tests. The GPU path has no headless test; it is exercised by the app,
the simulator launch and the page. That is the permitted
exercised-without-test quadrant; the M2 particle work inherits the same
split.

## 9. Exit

- [ ] Gate green; CI green.
- [ ] 120 Hz stable on the reference device: p99 display-link delta near
      8.33 ms over a minute, hitch count recorded.
- [ ] CPU and GPU frame time recorded with device and date.
- [ ] Idle: stats freeze when backgrounded or hidden.
- [ ] Power bound recorded.
- [ ] O2 budget proposed in HANDOFF from the measurements.
- [ ] Web page clears and reacts in desktop Chrome; a page with no WebGPU
      says so plainly.
