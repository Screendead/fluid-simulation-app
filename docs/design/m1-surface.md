# M1 — The surface

*Design record, 2026-08-30. Binds the code with `decisions.md`. Rewritten
the same day when the web target was removed (D1 amendment).*

## 1. Goal

wgpu enters. `fluid-core` owns a renderer. The renderer clears a
`CAMetalLayer` and presents it at display rate. The clear colour comes
from the body force, so a tilt of the phone shifts the colour. That proves
the path sensor → core → GPU → screen with no shader and no simulation.

The oracle: 120 Hz stable on the reference device, idle draw measured,
budget O2 set from measurements.

## 2. Architecture

`fluid-core` gains one public type, `Renderer`. It owns the wgpu device,
queue, surface and configuration, and the timing capture.

- `Renderer::new(instance, surface, width, height)` — requests the
  adapter and device, configures the surface. wgpu types cross this
  boundary; platform types do not. `fluid-ffi` makes the surface from a
  `CAMetalLayer` pointer (`SurfaceTargetUnsafe::CoreAnimationLayer`).
  wgpu's adapter and device futures are ready on the first poll (verified
  in the wgpu 30.0.1 source); a one-poll resolver keeps the API
  synchronous with no executor dependency.
- `Renderer::frame(sample, now_ms)` — encode one render pass that clears
  to the body-force colour, submit, present. No allocation. `now_ms` is
  `CADisplayLink.timestamp` in milliseconds; a gap over half a second is
  a pause, not a frame interval, and is not recorded.
- `Renderer::resize(width, height)` — reconfigure the surface.
- `Renderer::stats()` — percentiles from the timing rings. Called off the
  frame path, about once per second.

The shell owns the cadence; the core owns the frame. A `CADisplayLink`
pinned to 120 Hz drives `fluid_renderer_frame` through the C ABI
(`CADisableMinimumFrameDurationOnPhone` in the Info.plist).

## 3. Surface configuration

| Setting | Value | Why |
|---|---|---|
| Format | The surface's preferred format | No conversion cost |
| Present mode | `Fifo` | Vsync; the only universally supported mode |
| Frame latency | Start at 2; measure 1 against it | Each step of latency is one drawable of memory (~13.7 MB at native resolution) |
| Alpha | The surface's first supported mode (`Opaque` on Metal) | Nothing composites behind the box |
| Usage | `RENDER_ATTACHMENT` | A clear needs nothing else |
| Limits | `downlevel_defaults` raised to the adapter's resolution | WebGPU defaults overshoot small adapters; the simulator offers 15 inter-stage variables where the default asks 16 |

No depth buffer. No shader module, so the `wgsl` feature stays off; naga's
WGSL frontend enters at M2 with the first shader.

## 4. Timing capture

- CPU: wall time of encode+submit+present per frame, in a fixed ring of
  240 `f32` microseconds. This includes the drawable acquire, which blocks
  under back-pressure; M2 splits acquire from encode if the number needs
  reading more finely.
- GPU: `TIMESTAMP_QUERY` when the adapter offers it, two timestamps
  around the pass, resolved into a buffer and read back through a small
  ring of staging buffers with `map_async`. Never a blocking wait on the
  frame path. The reference device's adapter does not offer the feature
  (measured 2026-08-30: stats show zero), so the GPU column reads 0 until
  a finer probe exists; the A15 pipeline cost of a clear is far below the
  frame budget either way.
- Cadence: measured from `CADisplayLink` timestamps — the achieved rate,
  not the requested one.
- Memory: the shell reads `phys_footprint` and prints it in the stats
  line, one line per second, captured over `devicectl` launch console.

## 5. Idle

M1 idle means not visible. The shell pauses the display link when the
scene leaves the active state; no frame runs and the GPU does nothing.
Verified 2026-08-30 in the simulator: the stats counter froze while
backgrounded and resumed on foreground. Sleep-when-still is M6's work;
nothing here anticipates it (CLAUDE.md bans the scaffold).

## 6. Measurements (reference device, 2026-08-30, Release)

Sustained foreground run, ~96 s captured over console:

| Number | Value |
|---|---|
| Frame interval p50 / p99 | 8,334 µs / 8,334 µs — 120 Hz locked |
| Interval max (steady state) | 16,668 µs: a rare single dropped frame |
| Startup transient | ~2 s of hitches while the app comes up, then locked |
| CPU encode+submit+present p50 / p99 | ~1.4 ms / ~1.8 ms (includes drawable acquire back-pressure) |
| `phys_footprint` | 63.7 MB |
| Battery, thermal | 100%, nominal throughout |

The first battery-drain bound and the frame-latency-1 experiment move to
the heavy-optimisation pass Jack ordered after M2's visuals
(2026-08-30); O2 below is provisional until that pass firms it.

## 7. Dependencies

D4 in `decisions.md` records the set: wgpu 30.0.1 alone, default features
off, with `std`, `metal`, `parking_lot`. pollster, raw-window-handle and
a direct web-sys are all avoided; section 2 and D4 say how.

## 8. Tested and exercised

Pure pieces — the body-force-to-colour map and the percentile math — have
unit tests. The GPU path has no headless test; it is exercised by the app
and the simulator launch. That is the permitted exercised-without-test
quadrant; M2 inherits the same split.

## 9. Exit

- [x] Gate green.
- [x] 120 Hz stable on the reference device (section 6).
- [x] CPU frame time recorded with device and date (section 6); GPU
      timestamps unavailable on this adapter, recorded as such.
- [x] Idle: stats freeze when backgrounded (section 5).
- [ ] CI green on the pushed branch.
- [ ] O2 budget proposed in HANDOFF (provisional).
- [ ] Battery bound and latency-1 experiment: moved to the optimisation
      pass (section 6).
