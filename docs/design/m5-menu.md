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
| The look | `render.rs`, `sim_surface.wgsl`, `sim_sprites.wgsl` | Glass (M4) or flat: the colour or black, nothing between, and the charged strands as flecks of the colour. |

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

Jack, 2026-09-02, verbatim: "When I said "flat colour", I meant
"flat" - literally only two available colours for the screen: black,
and the chosen water colour. No blur, no fade, 0 or 1."

Jack, later the same day, verbatim: "So the binary colour scheme
means that tiny flecks end up disappearing. While remaining in
keeping with the binary colour scheme, can we still show the tiny
flecks of fast-moving water?"

The optics immediates grew from 48 to 64 bytes; the last vec4 is the
look: the water colour in linear light with a one in w, or zeros for
the glass. The surface shader returns the colour where the settled
thickness crosses the edge band's midpoint and black everywhere else,
before the wall lookup, so a flat pixel costs less than an air pixel.
There is no edge band: two colours, nothing between. The outline is
the blurred field's, the same waterline the glass reads; the blur is
the M4 wavelength filter against particle-footprint ripple, not an
edge softening.

The flecks are the glass look's strands drawn the binary way: a
tracer charged past the 0.05 m/s gate is one dot of the water
colour, written opaque and colour-only, so a dot over the body is the
body's colour and a dot in the air is a fleck of water in flight. A
still droplet too small for the threshold stays invisible, as it does
in the glass look; a moving one shows through its tracers. The tracer
compute runs in both looks. Rejected: a lower threshold in the flat
look, which fattens the whole outline to show pairs and still loses a
lone particle; per-particle discs, which would show every stranded
particle for ever.

The surface is `Bgra8UnormSrgb` on the reference device (logged
2026-09-02): the shaders work in linear light and the hardware
encodes on write. The core linearises the picker's components once,
at `set_look`, so the panel shows the bytes that were picked.

## The readout

| Line | Source | Colour |
|---|---|---|
| Frame rate | Frames stepped over the report interval; "idle" while the gate sleeps | green at 110 and above, amber at 55, red below |
| Temperature | `ProcessInfo.thermalState` | nominal green, fair yellow, serious orange, critical red |
| Frame cost | GPU p50 over the last 240 frames against 8.33 ms | green under 80% of budget, amber under 100%, red over |

At rest the GPU span reads the governor's clock, not the work
(optimisation record); the readout shows it as measured.

## Measured (reference device, 2026-09-02, 11:03 to 11:12)

Jack's hand on the phone through the 1x runs and the menu switches;
the pinned runs launched with `FLUID_SPACING`. Thermal state nominal
at the start, fair by the 4x run, serious by the 16x run: the session
heated the phone. Battery 100%, plugged.

| Build and state | Interval p50 / p99 | GPU p50 | Note |
|---|---|---|---|
| master a67522b, 1x glass, handled | 8,334 / 8,334 us | 6.13 to 6.40 ms | the before; 47 windows; 88 MB |
| m5, 1x glass, handled | 8,334 / 8,334 us | 6.14 to 6.46 ms | the after: the shader edit is neutral; 88 to 92 MB, 100 to 105 MB with the menu or the readout up |
| m5, 0.25x, pinned, at rest | 8,334 / 8,334 us | 2.26 to 2.28 ms | sleeps after 12 s; 74 MB awake, 64 MB asleep |
| m5, 4x, pinned, handled | 8,334 / 8,334 us | 6.4 to 6.6 ms | 120 Hz through 20 s of gentle handling; a hard shake dropped it to 40 to 60 Hz (GPU 10 to 25 ms) for 3 s, then it recovered; 88 to 105 MB |
| m5, 16x, from the menu, thermal fair | 9.3 to 16.7 ms p50 | 11 to 16 ms | the fresh lattice's first seconds; 144 MB at the rebuild, 112 MB after |
| m5, 16x, pinned, thermal serious | 120 to 163 ms p50 | 118 to 165 ms | six to eight frames a second: the substep basin; 125 MB |
| m5, 0.25x flat with flecks, handled then at rest (11:23) | 8,334 / 8,334 us | 6.3 to 6.7 ms in the hand, 2.7 ms at rest | slept 20 s in; 82 MB awake, 65 MB asleep |

The rebuild: 24 to 28 ms at every scale, from the menu (the
console's "sim: rebuilt in" line), pipelines included; Metal's
shader cache holds the compiled libraries. No pipeline/state split.

The GPU span under the governor: 1x, 4x and 0.25x all read 6.1 to
6.6 ms in the hand at 120 Hz, while 0.25x at rest reads 2.3 ms. The
A15 clocks the GPU to the work, so the span is the governor's
operating point whenever the frame fits, and it climbs past the
budget only when the work no longer fits. The readout's frame cost
is that signal: green while the frame fits, red when it does not.

The 16x basin: at 26,880 particles one substep costs about 9 ms, so
two substeps already overrun the 8.33 ms frame; the longer frame
floors the substep count at eight, and the frame settles near
120 ms. Real time at 16x is out of this GPU's reach by physics, as
the optimisation record said. The menu offers it because Jack asked,
rated bad.


The tap, the button, the menu, the scale switch and its 25 ms
rebuild, and the two-colour look were exercised by Jack's hand
during the captures (the fleck request came from his eye on the
first flat build); the flecks and the readout wait on his eye.

## Decisions

- This opens M5. The menu is the dropdown of Jack's 2026-08-30
  directive; the lenses land behind it later. Jack can overturn the
  naming.
- The scale rebuilds the whole sim, pipelines included: 24 to 28 ms
  measured, so no pipeline/state split.
- The ratings are static, from the measured ladder. A live rating
  can only rate the scale that is running.

## Tested and exercised

- `the_ladder_seeds_near_its_scales`: each scale seeds within 5% of
  its multiple of 1,620 on the reference screen, and 1x is the
  shipped spacing itself.
- `optics_immediates_land_at_the_shader_offsets`: the flat look lands
  at byte 48; `the_flat_colour_is_linearised`: hot pink through the
  sRGB curve, the glass all zeros.
- `the_settled_field_matches_the_calibration`: the plateau at two
  spacings.
- Every shell path is exercised by the menu; the shell has no test
  target.
