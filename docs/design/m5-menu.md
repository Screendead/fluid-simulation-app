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
| The look | `render.rs`, `sim_surface.wgsl`, `sim_sprites.wgsl` | Glass (M4), or flat on black in two views: the surface, two colours and nothing between; or the particles alone, each a disc of the colour. |

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
absorption, and the edge band follow it. The band is now 0.15 and
0.30 of the settled field, which is 0.8 and 1.6 of the raw field at
the shipped spacing: the same water at every scale. Two tests
measure the plateau, one at the shipped spacing to pin the constant
and one at 0.0063 m to pin the scaling, each to 10%.

## The flat looks

Jack, 2026-09-02, verbatim: "When I said "flat colour", I meant
"flat" - literally only two available colours for the screen: black,
and the chosen water colour. No blur, no fade, 0 or 1."

The optics immediates grew from 48 to 64 bytes; the last vec4 is the
look: the water colour in linear light with a one in w, or zeros for
the glass. The surface shader returns the colour where the settled
thickness crosses the edge band's midpoint and black everywhere else,
before the wall lookup, so a flat pixel costs less than an air pixel.
There is no edge band: two colours, nothing between. The outline is
the blurred field's, the same waterline the glass reads; the blur is
the M4 wavelength filter against particle-footprint ripple, not an
edge softening.

The surface is `Bgra8UnormSrgb` on the reference device (logged
2026-09-02): the shaders work in linear light and the hardware
encodes on write. The core linearises the picker's components once,
at `set_look`, so the panel shows the bytes that were picked. The
format is `caps.formats[0]`, which wgpu sorts sRGB first; the log
line prints it every launch.

### The particle view

The flat surface cannot show a lone drop: one particle does not
cross the threshold, so it is black. Jack asked for the fast flecks
back, and two builds put them on top of the surface, first the
charged tracers as single dots, then every particle as a small disc.
His eye on the first, verbatim: "no there are way too many, the
problems: - too much flecking, it sometimes looks consistently dense
even quite far away from the bulk of the water and even when it's
not moving much - there are always little stragglers even when the
fluid is at rest - ultimately it looks like two different things are
being rendered, rather than one single "water" pass with the flecks
included - the flecks are too small, too". His ruling, verbatim:
"actually just make the single-particle view toggleable when in the
flat view mode, don't combine them. leave the fluid view as it was
before."

So flat holds two views and a toggle chooses; glass is untouched.
The particle view is the particles alone: each one a disc of the
water colour on black, written opaque and colour-only. Nothing
massless is drawn, so nothing strays at rest.

### The size of a disc

Jack, 2026-09-02, verbatim: "Make the discs the same size they
currently are *when they're within a body of water*, and make them
smaller in proportion to how far away they are from other discs,
especially when they're on their own. They should never be 1px but
still they should go smaller in the "balls" view".

The measure of a neighbourhood is density, and the solver already
carries it per particle. Measured on a settled slab (this machine,
2026-09-02, 600 frames upright, the test below):

| Where a particle is | Density, of rest density |
|---|---|
| Body and free surface, the 95% | 0.996 to 1.004 |
| The outermost fringe | 0.69 |
| A touching pair | 0.26 |
| Alone | 0.184, its own kernel weight |

So the disc holds its full half of h at and above 0.65 of rest
density, which covers a resting body to its last particle, and falls
linearly to the floor at 0.25, which every detached drop is already
below. Between them sit the splashes, and they shrink with how
detached they are. The floor is three device pixels of the drawable,
passed in metres in the pass's immediates: a dot, never a pixel. The
first frame after a launch or a resume draws every disc at the floor,
the solver not having run yet.

At full size the discs of a resting body overlap and read solid, and
a lone drop is a dot; full size is about 12 mm in the modelled tank
at 1x and scales with the spacing.

The view builds no thickness field at all: the decay, the
per-particle splat, the blur and the surface pass go unencoded, and
the pass clears to black instead of the dazzle backdrop. The field
is stale while the view runs, so the first field frame after it
decays the old field to nothing and splats the whole weight, which
is the same steady state and no ghost of the shape a minute ago.
The launch frame takes the same path, the field texture being
zeroed, so the glass look now reaches its first frame without the
old ramp-in; the two are the same picture after one frame.

Neither flat view advects or draws the tracers. Returning to glass,
the strands regather over the 3 s respawn constant.

A look change restarts the idle gate. A sleeping sim presents no
frame, so a toggle or a picker drag on a still phone would otherwise
not reach the screen (found in review, 2026-09-02).

Rejected: the two views layered, by Jack's ruling above; a lower
threshold on the surface, which fattens the whole outline to show
pairs and still loses a lone particle; the charged tracers as dots,
which draw the massless cloud rather than the water.

## The readout

| Line | Source | Colour |
|---|---|---|
| Frame rate | Frames stepped over the report interval; "idle" while the gate sleeps | green at 110 and above, amber at 55, red below |
| Temperature | `ProcessInfo.thermalState` | nominal green, fair yellow, serious orange, critical red |
| Frame cost | GPU p50 over the last 240 frames against 8.33 ms | green under 80% of budget, amber under 100%, red over |

At rest the GPU span reads the governor's clock, not the work
(optimisation record); the readout shows it as measured.

The readout costs the frame nothing measurable. With all three
lines on, the GPU span, the CPU encode and the stats call itself
(about 110 us once a second) all read the same as with them off;
the overlay holds about 4 MB. The stats call runs either way for the
console line.

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
| m5, 0.25x flat with the flecks layered, handled then at rest (11:23) | 8,334 / 8,334 us | 6.3 to 6.7 ms in the hand, 2.7 ms at rest | the rejected layered build; slept 20 s in; 82 MB awake, 65 MB asleep |
| m5, 1x glass, still on the desk (16:32) | 8,334 / 8,334 us | 6.34 to 6.50 ms | the three-look run; 19 windows, then it slept; 85 MB |
| m5, 1x flat surface, same (16:33) | 8,334 / 8,334 us | 3.15 to 3.28 ms | 73 to 78 MB |
| m5, 1x particle view, same (16:34) | 8,334 / 8,334 us | 2.22 to 2.39 ms | 64 to 67 MB; CPU encode 0.96 ms against the glass run's 1.34 ms |
| m5, 1x glass, all three readout lines on, same (16:39) | 8,334 / 8,334 us | 6.33 to 6.50 ms | the same span as the glass run without them; CPU encode 1.33 ms against 1.34 ms; 90 MB against 85 MB |
| m5, 1x particle view, discs sized by density, at rest then handled (17:41) | 8,334 us p50 throughout | 2.20 to 2.45 ms at rest, 3.3 to 6.0 ms handled | the density read costs nothing: the resting span matches the fixed-size run; the handled span is the solver's substeps, which every look pays; 72 to 75 MB |

The three-look run: the same build launched three times with the
spacing pinned to 0.01 m and the look set from the launch arguments,
the phone still on the desk each time. Every run fell, settled and
slept, and all three held 120 Hz throughout. The order is the pass
count. Glass builds the field, blurs it, fills the screen and advects
and draws the strands; the flat surface builds the field, blurs it
and fills; the particle view draws the particles and nothing else.
The governor caveat above still applies to the absolute numbers, but
the three ran back to back on a nominal-thermal phone at the same
scale.

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
rebuild, and the two-colour surface were exercised by Jack's hand
during the captures; both fleck builds came from his eye on the one
before. The particle view ran on the phone in the three-look run; its
look, and the readout, wait on Jack's eye.

## Decisions

- This opens M5. The menu is the dropdown of Jack's 2026-08-30
  directive; the lenses land behind it later. Jack can overturn the
  naming.
- The scale rebuilds the whole sim, pipelines included: 24 to 28 ms
  measured, so no pipeline/state split.
- The ratings are static, from the measured ladder. A live rating
  can only rate the scale that is running.
- The particle view is a view of the flat look, not a third top-level
  choice: it shares the colour, and glass keeps the M4 pipeline
  untouched.
- The particle view skips the field passes rather than drawing over
  them. Two colours are two colours either way, and the skipped
  passes are the frame's cheapest way to draw the water it shows.
- A disc's size reads the solver's density, not a neighbour count or
  a distance search: the number is already computed every substep, so
  the law costs one buffer read in the vertex shader. Rejected: a
  count of neighbours, which is the same buffer traffic and a coarser
  signal; a screen-space measure, which needs a pass of its own.

## Tested and exercised

- `the_ladder_seeds_near_its_scales`: each scale seeds within 5% of
  its multiple of 1,620 on the reference screen, and 1x is the
  shipped spacing itself.
- `optics_immediates_land_at_the_shader_offsets`: the flat look lands
  at byte 48; `the_flat_colour_is_linearised`: hot pink through the
  sRGB curve, the same word for both flat views, the glass all zeros.
- `the_settled_field_matches_the_calibration`: the plateau at the
  shipped spacing; `the_settled_field_scales_with_the_spacing`: the
  plateau at 0.63 of it, against the prediction.
- `the_particle_view_draws_two_colours`: the disc pass over a black
  clear reads back magenta or black and nothing else.
- `the_settled_body_keeps_its_discs`: a settled slab's fifth
  percentile density sits in the law's full-size plateau, and a lone
  particle's density sits under its floor. The two ends of the disc
  law, measured rather than assumed.
- Every shell path is exercised by the menu; the shell has no test
  target.
