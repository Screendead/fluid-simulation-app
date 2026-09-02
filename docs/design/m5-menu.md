# M5 — The menu

*Design record for M5, opened 2026-09-02. Binds the code. Amend by
explicit edit. The roadmap's M5 row is the field lenses behind a
dropdown; this record opens the menu they will sit in. The lenses
come later, in this record.*

## The directive

Jack, 2026-09-02, verbatim: "I want to be able to tap to bring up a
button to open a menu/modal, in which I can choose how to render the
liquid. I should be able to choose 0.25x, 1x, 4x, or 16x particle
count (with coloured indicators showing how my phone will perform -
good, borderline, or bad for anything non-realtime). I should be able
to toggle "flat mode" where the background goes black and the liquid
just shows as a flat colour (default: hot pink). I should be able to
choose that colour. I also want to be able to toggle showing current
FPS, phone temperature rating, and current ms/frame vs budget
ms/frame."

## What it builds

The split is D6: the shell owns the controls and remembers the
choices; the core takes a particle scale and a look and computes
everything about the fluid.

| Part | Where | What |
|---|---|---|
| The tap and the button | `FluidApp.swift` | A tap anywhere shows one button for four seconds. The button opens the menu as a half sheet, so the water stays in view while a toggle changes it. |
| The menu | `Menu.swift` | Three sections: particles, look, readout. Every choice persists in `UserDefaults` and comes back at the next launch. |
| The readout | `Readout.swift` | Frame rate, thermal state, and GPU time against the 8.33 ms budget, one line each, updated once a second from the stats call the console line already makes. |
| The particle scale | `render.rs`, `set_particles` | The spacing that seeds nearest the scale times the shipped count, then a rebuild of the sim. |
| The look | `render.rs`, `sim_surface.wgsl` | Glass (M4) or flat: one colour on black, no strands. |

Defaults: the button hidden, the readout off, glass, 1x. The GUI-free
screen of 2026-09-01 is the default state.

## The ladder

Counts on the reference device's screen (1284 x 2778 px, the 4x
world). The lattice quantises in whole rows and layers, so the core
searches the spacing in steps of half a percent instead of taking a
cube root: the cube root lands 0.25x at 288 particles and 16x at
32,340. One count spans a run of spacings; the run's spacing nearest
the cube root wins, so 1x is the shipped spacing itself. The rating
is a table in the shell keyed on the measurement in the next section;
a second device would bring its own row.

| Scale | Spacing | Particles | Layers | Cells | Rating |
|---|---|---|---|---|---|
| 0.25x | 0.0142 m | 399 | 1 | 693 | good |
| 1x | 0.0100 m | 1,620 | 2 | 1,568 | good |
| 4x | 0.0062 m | 6,468 | 3 | 4,840 | borderline |
| 16x | 0.00435 m | 26,880 | 6 | 9,300 | bad |

`FLUID_SPACING` still pins the spacing for a measurement run; the
scale applies when it is unset, and the menu's choice replaces either.

## The calibration at every scale

`FIELD_SETTLED` (5.3) was measured at the shipped spacing. The
settled field is the particle layers per screen area: the fluid
fills the slab depth at rest density, so the field scales with
`SLAB_DEPTH / spacing`, and the sim carries
`FIELD_SETTLED * SIM_SPACING / spacing`. The thickness, the
absorption, and the edge band (now 0.8 and 1.6 settled units, the
same water at every scale) follow it. The calibration test measures
the plateau at the shipped spacing and at 0.0063 m, and pins the
constant and the scaling to 10%.

## The flat look

The optics immediates had one spare word; it carries the look as
RGBA8, alpha nonzero for the flat colour and zero for glass. The
surface shader returns the colour times the edge band before the
wall lookup, so a flat pixel costs less than an air pixel. The
tracer compute and the strand draw are skipped: flat water is one
colour. Returning to glass, the strands regather over the 3 s
respawn constant.

The colour passes through in the display's own space: the surface
is `Bgra8Unorm` on the reference device (logged 2026-09-02), so the
picker's components are the bytes the panel shows.

## The readout

| Line | Source | Colour |
|---|---|---|
| Frame rate | Frames stepped over the report interval; "idle" while the gate sleeps | green at 110 and above, amber at 55, red below |
| Temperature | `ProcessInfo.thermalState` | nominal green, fair yellow, serious orange, critical red |
| Frame cost | GPU p50 over the last 240 frames against 8.33 ms | green under 80% of budget, amber under 100%, red over |

At rest the GPU span reads the governor's clock, not the work
(optimisation record); the readout shows it as measured.

## Measured (reference device, 2026-09-02)

Filled in by the device session below.

## Decisions

- This opens M5. The menu is the dropdown of Jack's 2026-08-30
  directive; the lenses land behind it later. Jack can overturn the
  naming.
- The scale rebuilds the whole sim, pipelines included. Measured
  below; a pipeline/state split waits on that number.
- The ratings are static, from the measured ladder. A live rating
  can only rate the scale that is running.

## Tested and exercised

- `the_ladder_seeds_near_its_scales`: each scale seeds within 5% of
  its multiple of 1,620 on the reference screen, and 1x is the
  shipped spacing itself.
- `optics_immediates_land_at_the_shader_offsets`: the look word lands
  at byte 44 as RGBA8.
- `the_settled_field_matches_the_calibration`: the plateau at two
  spacings.
- Every shell path is exercised by the menu; the shell has no test
  target.
