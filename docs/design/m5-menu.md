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

The 4x dip, explained (2026-09-02, evening): the "hard shake dropped
it to 40 to 60 Hz for 3 s" in the 4x row is a pacing feedback, not the
GPU's limit. Measured the same evening with a same-run alternating
instrument (optimisation record, "The 4x session"): a five-pass
substep at 4x costs 0.97 to 1.0 ms cool and 1.25 to 1.36 hot, the
glass look adds 2.4 ms a frame of its own, and five substeps still
lock 120 Hz; six or seven do not, the frame slips, and the CFL and
the refine rung both read the slipped frame until the count rails at
the cap.

Measured on the device 2026-09-03, on branch `m5-4x` (optimisation
record, "The device session"): the dip is gone. Jack swirled the phone
briskly for 70 seconds a look, hot, at 4x. The median second holds
8,334 us on both the flat and the particle look; the longest unbroken
stretch over budget is 2 seconds on flat and 4 on particles, against
the 45 to 55 Hz the old dip held until v_max fell under 1.5 m/s. The
upright boil is gone with it: no CFL clamp after the first five
seconds, where the shipped cap clamped 4,280 times a second without
end. The condition this paragraph set is therefore met, and the
4x row is rated `good`. Jack's rule for the rating, 2026-09-03: "my
aesthetic opinion is separate from the objective performance claim; an
option is green if it holds 120fps on the phone." So the flat look's
own fault at 4x does not hold the rating down: the particle look at 4x
is "stunning" in his words and the flat look is not, because its
flecks "just seem to teleport", and that is the field path's fault,
not the pacing's (HANDOFF O7). Two findings from the same evening
bind this record's 4x row: the 4x pool held upright and still boiled
at the shipped substep cap (Jack: "it absolutely does jitter/boil at
4x"), and Jack deprioritised the glass look at 4x ("flat and
particle look the coolest").


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

## The touch

Jack, 2026-09-02: "make it possible to drag to peturb the sim, and if
possible, make it so that the force of my touch peturbs it with a
wider net. It shouldn't repel; it should drag the water as if I were
putting my finger through it."

A finger on the glass entrains the water it crosses. Inside a disc of
`Finger::RADIUS` through the whole slab, the in-plane velocity is
pulled towards the finger's own velocity:

```
v.xy <- mix(v.xy, finger_v, 1 - exp(-TOUCH_RATE * w * w * dt))
w = 1 - (distance / radius)^2
```

Four properties follow from that form, and each is what Jack asked
for:

| Property | Why the form gives it |
|---|---|
| It drags, never repels | The target is the finger's velocity, not a direction away from it. Water on both sides of the stroke moves the same way. |
| Every finger drags | Jack, the same day: "Multi-touch should behave the same way for each simultaneous finger." Five fingers, each with its own position and velocity. |
| A wider net | The radius is 25 mm of glass, which is 0.1 m of the modeled tank at `WORLD_SCALE` 4 — about a third of the screen's width. |
| A still finger brakes | With a zero finger velocity the blend damps. A real finger held in water does the same. No finger at all is a zero radius, which is a different state. |
| Pacing cannot move it | `TOUCH_RATE` is a rate per second, exponentiated over the substep, the same idiom `XSPH_RATE` uses. |

Only the screen plane is entrained. The finger has no depth, and
pulling z to zero would flatten the flow it stirs.

### Many fingers at once

Water inside more than one finger's disc goes towards the weighted
average of their velocities, and goes there faster: the weights sum
into the rate.

```
pull   = sum over fingers of w_i * v_i
weight = sum over fingers of w_i
v.xy  <- mix(v.xy, pull / weight, 1 - exp(-TOUCH_RATE * weight * dt))
```

Summing before the blend, rather than blending once per finger, is
what makes two fingers on one spot behave the same whichever order the
loop found them in. The blend factor is `1 - exp(-x)`, which is under
one for every `x`, so no weight can overshoot and the sum needs no cap.

Five slots, the number of fingers the phone reports. The shell owns
the identity — only it can tell one finger from another — and holds a
slot for each finger until it leaves the glass. The core keeps one
tracker per slot and packs the live ones to the front of the block, so
the solver's loop costs nothing for fingers that are not down.

Rejected: a storage buffer of fingers, which needs a binding and a
write a frame to carry at most eighty bytes; and re-packing the
trackers when a finger lifts, which would hand a lifted finger's
speed to whichever finger took its place.

### Where each part lives

| Part | Where |
|---|---|
| The fingers, their slots, and their points on the drawable | `MetalLayerView` in `platforms/ios/Sources/FluidSurface.swift` |
| Normalised point across the ABI | `fluid_renderer_touch(renderer, slot, x, y, down)`, with `FLUID_TOUCH_SLOTS` for the slot count |
| Point to metres, and each finger's speed | `Fingers` in `render.rs` |
| The entrainment | `fingers()` in `sim_solve.wgsl`, inside `forces_den_apply` |

SwiftUI's `DragGesture` reports one finger, so the touch handling sits
in the `UIView` that already owns the Metal layer: `touchesBegan`,
`touchesMoved`, `touchesEnded` and `touchesCancelled`, with
`isMultipleTouchEnabled`.

The shell reports a point on its own drawable and nothing else, which
is D6's split. The core flips y, scales by the box extent, and
differences the point across its own frames: the shell never computes
a velocity.

The tap that reveals the menu button is now the end of a drag that
went nowhere — under 10 points of travel. One gesture, so a stroke
through the water can never open the menu.

### Dials

| Dial | Value | Why that value |
|---|---|---|
| `Fingers::RADIUS` | `WORLD_SCALE * 0.025` | 25 mm of glass, a fingertip and the halo around it |
| `TOUCH_RATE` | 40 per second | Water under the middle of the finger reaches its speed in about 25 ms |
| `Fingers::SMOOTH_TAU` | 20 ms | Touches and frames run at the same rate out of phase, so the raw per-frame difference alternates between a doubled step and none |

All three are Jack's to move once he has felt it. None is derived
from physics: a finger is not a physical model here, it is a control.

### The idle gate

A finger down clears the gate and holds it clear. Without that, a
still phone asleep on a desk would ignore the touch entirely: the
frame returns before any solver work is encoded.

## The lenses

Jack, 2026-09-02: "make the color a gradient - the user should be able
to choose a simple single colour or optionally two colours denoting
low->high values. The colours should then be applied to a metric of the
user's configuration; one of [velocity, acceleration, pressure,
proximity] (add others if you think they'd be cool; use discernment)."

One colour is the flat look unchanged. Two colours make a ramp across
one field, low to high, and every field is one the solver already
carries, so four of the five lenses cost no pass of their own. The
direction wheel is the exception and pays for the pass it needs.

| Lens | The scalar | Where it comes from |
|---|---|---|
| Velocity | speed, m/s | `length(velocities[i].xyz)` |
| Acceleration | m/s² | `velocities[i].w`, a running mean `integrate` writes |
| Pressure | Pa | `prev_vel[i].w`, the same mean of the solver's `pressure` |
| Proximity | density against rest | the solver's `density` |
| Direction | which way, as a hue | `atan2` of the in-plane velocity |

The fifth was temperature until Jack looked at it, 2026-09-02: it
"doesn't really show anything interesting. Temp doesn't change much,
if at all, and it just looks like random dappling mostly since the
temps of nearby particles don't correlate with each other." He is
right, and the arithmetic says why: a settled box spreads 1.5
millikelvin end to end, while a float near 293 K resolves 30
microkelvin, so the lens painted about fifty quantisation steps. The
direction wheel takes its number. Temperature stays in the solver and
on the readout, where a drift over minutes is the point.

Rejected: vorticity. It is the most interesting field the app does not
have, and the only one on this page that needs work of its own — a
neighbour sweep per particle, or a curl of the splatted velocity grid,
which exists only while the strands run and only at the coarse cell
size. Worth building when Jack asks; not worth a sweep smuggled in
behind a colour picker.

The direction wheel moved that price. Its second field is a
kernel-weighted sum of unit headings over the same footprint the
thickness uses, and splatting the velocity itself instead of its
direction would make it a smoothed velocity field, whose curl is one
texel difference away in the fill. That is vorticity on the flat
surface for the cost of a subtraction — not on the particle view,
which builds no field, and not free, because the wheel's own pass is
what pays for it.

### Acceleration, and where it comes from

The other four fields sit in a buffer already. Acceleration did not,
and the honest number is the whole velocity change over a substep —
body force, viscosity, tension and every pressure iteration together.
That total exists in exactly one place, the top of `integrate`, and
only by difference against where the substep started.

So `integrate` keeps the previous substep's velocity in `prev_vel` and
writes the magnitude of the change into `velocities[].w`, a slot every
writer set to zero and nothing read. Those writers now carry it
through instead, which they must: a slot two dispatches zero every
substep cannot hold a running mean. No new binding reaches the draw.

Two details are deliberate. The difference is taken before the wall
zeroes a contact, so the boundary's own impulse does not draw an
outline of the box. And it is not taken in `forces_den_apply`, where
the velocity is in registers already: `kd` is half-warm-started there,
so a settled column would read half a gravity everywhere instead of
nothing.

### The ranges

Jack, 2026-09-02: "The gradient should go from the lowest value
*actually present in the sim* to the highest *actually present*. Not
from some absolute number to another."

So every ramp spans the frame's own two ends. `reduce_stats` already
reduced four of the five fields; it now hands back a low-high pair for
each, in the two rounds that were carrying one and a half each. The
reduction lanes were the constraint: `reduce_pair` sums one lane and
takes extrema in three, which is one sum and three extrema a round,
and eight extrema were wanted. `reduce_bounds` is the second helper —
two low lanes, two high lanes, two lenses a round — and it needs no
round and no buffer read that was not there already, because velocity
and acceleration come out of one fetch of `velocities[i]`.

| Lens | Low | High |
|---|---|---|
| Velocity | slowest particle | fastest |
| Acceleration | least | most |
| Pressure | least | most |
| Proximity | sparsest neighbourhood | fullest |
| Direction | the velocity pair; the hue rides on it |

**A floor under the span.** A ramp between two ends that are almost
the same number is a ramp across the solver's own noise. A settled
pool spans 0.02 m/s of speed, and every particle walks a ninth of
that from one frame to the next, which paints confetti. Each lens carries the
narrowest span worth a ramp, and three of the four are the quantity
one particle spacing of water holds, so they scale with the ladder.

| Lens | Floor | What it is | A settled pool |
|---|---|---|---|
| Velocity, Direction | `sqrt(2 g d)`, 0.44 m/s | the speed of a fall through one spacing | 0.02, so the floor holds |
| Acceleration | `g` | the fall itself | 195, well over |
| Pressure | `rho0 g d`, 98 Pa | the weight of one spacing of water | 7,100, well over |
| Proximity | `0.01 rho0`, 10 | a percent of rest density | 280 across the pool, 4 across the settled body alone |

The floor binds on a still pool and nowhere else, which is what it is
for. It is also the guard against a zero span, so it is stated for
every lens, not only the one that needs it today.

**The ends are chased, not taken.** A frame's own extremes walk: the
settled pool's speed ceiling moves a seventh of its own span every
frame, and pressure's a fortieth, and any of that shifts every colour
on the screen at once. `Ramp` holds the live lens's two ends and
follows the frame with `1 - exp(-dt/tau)`. Opening out takes 0.15 s,
so a splash has its colours inside a third of a second; closing in
takes 0.6 s, so the palette does not breathe every time the fastest
particle slows down. The ramp resets when the lens changes and holds
while the pool sleeps.

Nothing is derived from the box any more. The old anchors — a fall
down the column, twice hydrostatic, the disc law's two ends — were
each derived and each defensible, and the pressure one carried a real
finding: the solver's pressure is not the column's weight, because
`forces_eval` opens each substep at half the last one's pressure and
the solve adds its corrections on top, so a converged substep settles
at twice what it corrects. That is still what the pressure lens
*means*. It is no longer where the ramp ends.

### The shimmer

Jack, 2026-09-02: "the water is a bit flickery with the gradient on,
at near-rest."

The measurement is how far a settled particle walks along its ramp
between two frames, averaged over every particle, from a headless run
of 30 frames after a 600-frame settle
(`a_settled_pool_holds_its_colours_still`, this machine). One number a
lens, and it ranked the causes in one run.

| Lens | Before | After |
|---|---|---|
| Velocity | 0.09% | 0.52% |
| Acceleration | **19.4%** | 0.09% |
| Pressure | 2.2% | 0.06% |
| Proximity | 0.01% | 0.13% |
| Direction | — | 0.02% |

The acceleration lens was the flicker, by a factor of nine over the
next worst. Its raw number is the pressure solve's residual as much as
the flow: a settled pool reads 20 g on one particle while the pool
around it reads a tenth of that. The tail is not a transient: the
running mean holds it, which points at the wall, where a particle in
contact is zeroed and re-accelerated every substep. That is inference
from the numbers, not a measurement of its own. So
both that lens and pressure now read a running mean over 50 ms, in the
same `1 - exp(-rate·dt)` form the finger and XSPH use, so the substep
count cannot change it. The mean costs one `mix` each: `integrate`
already has both values in registers, and both slots were free —
`velocities[].w` was already the lens's, and `prev_vel[].w` was
written as zero and read by nothing. Two other writers of `velocities`
had to stop zeroing `w`, which they do by carrying it through.

The device shows the same thing from the other side. A settled pool
under the glass reports a pressure ceiling of 1,331 Pa on the build
before this one and 1,101 on this one (reference device, 2026-09-02,
the same still desk): the raw peak sat a fifth above the field the
mean paints, and that fifth was arriving on a different particle
every frame.

**The readout's pressure moved with the lens.** `RenderStats`'
`pressure_min` and `pressure_max`, and the `p ..Pa` field of the
console line, now report the running mean rather than the substep's
raw pressure — one reduction, not two, and the mean is the better
number for the order-of-magnitude check that field is for. The
compression pair is untouched and stays raw, and that is the pair
that watches the solver converge.

Velocity and proximity got noisier, and that is the price of Jack's
ask: the ramp is tighter now, so the same jitter covers more of it.
Both stay an order of magnitude under the acceleration lens's old
number, and velocity's 0.52% is the span floor doing its work: without
it the number is 11%.

### The wheel

Jack, 2026-09-02: "What *would* be cool is a hue-wheel rainbow
colouring based on the theta of the velocity!"

The hue is `atan2(v.y, v.x)`, the whole wheel over a turn. The chosen
low colour is what still water reads, and the hue takes over as the
**square** of where the water's speed sits on the velocity ramp.

The square is not decoration. The direction of water at rest is noise
— a uniform random angle every frame — so a linear mix smears that
noise over the whole pool: 1.15% of colour movement a frame on a
settled pool, against 0.02% squared. It also reads better, because the
wheel is then saying something about the water that is moving instead
of tinting everything.

Where the wheel starts is `atan2(v.y, v.x) / TAU + 0.5`, which puts
red on water running left, cyan on water running right, yellow-green
on water falling and blue on water rising. One added constant turns
the whole wheel if Jack wants a different pairing.

The high colour goes unread, and the menu hides its picker for this
lens.

**Two numbers down one interpolant.** The disc needs a hue and a
saturation where a ramp needs one number, and a varying of its own
costs 490 microseconds a frame (below). Both ride in clip `z`: the
saturation in ten bits above the point, the hue below it. Every vertex
of a quad carries the same value, so the fragment reads the pair back
whole.

The hue takes the fraction and not the other way round, because "reads
it back whole" holds only while the interpolator returns the vertex
value bit for bit. Put the hue below the point and a value that lands
one unit the wrong side of a step carries a hue wrapped a whole turn,
which is the colour it already was. Put the saturation there and the
same slip drops a disc to the low colour, which reads as a twinkle in
fast water. The test cannot tell those apart — a black disc looks like
air — so the packing is ordered to make the failure invisible instead.

**An angle has no mean.** The flat surface takes a kernel-weighted
mean of the lens and would average headings across the seam at half a
turn — a pool sloshing left has half its particles at +3.13 radians
and half at -3.13, and the mean of those is the opposite colour. So
the direction lens splats a second field of its own: each particle
adds its unit heading vector, and the surface reads the heading of the
sum, which is right everywhere. The saturation comes from the ordinary
lens channel, which for this lens is the speed.

That second field is an `Rg16Float` of the same size as the first, one
more decay draw and one more splat, and it is written only while the
lens is on. Widening the one field to four channels was the
alternative and it was rejected: it would tax the glass look, which
never reads a lens, on every frame.

### Where the colour is applied

| Look | How |
|---|---|
| Particle view | Each disc takes its own particle's place on the ramp, flat-interpolated. One buffer read in the vertex stage. |
| Flat surface | The body splat writes the lens into the field's second channel, weighted by the same kernel footprint as the thickness. The fill divides one by the other. The direction lens adds a second field of headings. |
| Glass | Untouched. |

The field texture is `Rg16Float` instead of `R16Float`: r the
thickness, g that thickness times the lens. Both channels decay
together through the same blend, so the ratio survives the field's
frame-to-frame average, and r is bit-for-bit what it was —
`FIELD_SETTLED` and the edge band did not move, which
`the_settled_field_matches_the_calibration` still shows.

The flat look reads that ratio from the raw splat, not from the blur.
A kernel-weighted mean over a 1.5 h footprint is already smooth inside
the body, and the blur exists for the caustics, which the flat look
never runs. That saves a second blurred channel and a second filter
pipeline. The threshold that decides water from air runs first, so the
divide never sees a thickness near zero.

**A varying is not free on the A15.** The disc first carried its
colour to the fragment as one flat `vec3`. That cost 490 microseconds a
frame at 1,620 discs — 877 to 1,368 on the render pass, with the
compute pass untouched at 1,350 (reference device, 2026-09-02, per-pass
timestamps, particle view on a still desk). Neither the acceleration
buffer, the nineteenth storage binding, the two new sprite bindings nor
the lens read itself moved the number; removing the varying restored it
exactly.

That finding has a sibling, found the same day and recorded under
"Measured again": a branch costs what it contains, whether or not the
draw takes it. Both are the same lesson — on this GPU a draw is priced
by what its entry point *can* do, not by what it does.

Both passes carry the ramp in clip `z` instead. Neither has a depth
attachment, so `z` is an interpolant already paid for, and every vertex
of a quad carries the same value, so the fragment reads it back exactly.
The lens is clamped to 0 to 1, which is the whole clip range, so no
primitive can be clipped by it. With that, the lens costs nothing
measurable on the particle view: 2,248 microseconds against the
pre-lens 2,240 and the pre-drag 2,439. On the flat surface it is not
free, which nothing measured until "Measured again" below.

The glass look stays glass. Jack's directive of 2026-08-30 makes the
water renderer the default view and puts every field lens behind the
menu; a metric tint over real refraction would fight both. A vertex
branch on the high colour's w keeps the glass from paying for the lens
buffers at all.

**This amends the flat look's rule above.** "Literally only two
available colours for the screen: black, and the chosen water colour"
still holds with one colour chosen, which is the default. With two, the
body carries the ramp between them, and under the direction wheel it
carries the wheel's own hues, which Jack asked for by name. The hard
water/air edge, the part of the rule that is about the look rather
than the count, is unchanged: no blur, no fade, water or black.

## Measured (reference device, 2026-09-02, evening)

Particle view at 1x, phone flat and still on a desk, `devicectl`
console over the fall-and-settle and into the idle gate. One build a
row, each launched cold.

| Build | GPU p50 settled | Frame interval |
|---|---|---|
| Before the drag (`4c4436d`) | 2,439 µs | 120 Hz |
| Drag and multi-touch (`24d01f5`) | 2,240 µs | 120 Hz |
| Lenses, with the disc varying | 2,723 µs | 120 Hz |
| Lenses, ramp in clip z | 2,248 µs | 120 Hz |

Both features are free. The 2,240 against 2,439 is a small unexplained
win in the drag commit, reproduced twice; it is not claimed as one.

Every row above is the particle view. The glass look runs the field
splat, the filter and the fill, and the lens commit changed all three
— the field texture is `Rg16Float` now, the fill binds a third texture,
and the optics block is 80 bytes. Same protocol, same evening:

| Build | GPU p50 settled, glass |
|---|---|
| Before the drag (`4c4436d`) | 6,428 µs |
| Drag, multi-touch and lenses (`31349e4`) | 6,504 µs |

76 microseconds, 1.2%, which is the second channel of a quarter-
resolution field written once and read once a frame. The glass look
never reads it: a ramp needs a flat look, and the body vertex branches
on the high colour before it touches a lens buffer.

The two-span split above came from a throwaway build that printed the
compute and render timestamps apart instead of summing them. That is
the tool to reach for again: the summed number said only that a frame
had grown, and four wrong hypotheses died before the split named the
pass in one run.

## Measured again (reference device, 2026-09-02, late)

Three protocol changes, each from a number read wrong first.

**`FLUID_LOOK` names the look.** The menu is the only other way in and
a console run cannot reach it, so before this every look but the
stored one was out of reach. The console line now names the look it
measured, gradient and lens included.

**The settled cost is the p50 the ring reports while the frame counter
is frozen.** Two wrong readings preceded it. The last line reports
whatever the desk was doing at the end, and said that removing three
`pow`s made a shader slower. The run's lowest p50 reports the fall
rather than the settle, and on the particle view the fall is the
cheaper of the two, so it reads a number the water never holds. While
the gate has the pool the window holds settled frames and nothing
else, so the longest frozen run is the number.

**A cross-sweep delta of 200 microseconds means nothing.** The same
build measured an hour apart moved by that much on the flat look while
the glass held to 65, so a claim about a code change needs its two
builds measured back to back, installed one after the other. Every
causal number below is a pair taken that way. The table is the
shipping build's own cost, not a difference.

One launch a row, 100 seconds, phone flat and still on the desk, 1x.

| Look | GPU p50 settled | Of the 8,333 µs budget |
|---|---|---|
| Glass | 6,430 | 77% |
| Flat, one colour | 3,586 | 43% |
| Flat + velocity ramp | 3,733 | 45% |
| Flat + direction wheel | 4,968 | 60% |
| Particles, one colour | 2,554 | 31% |
| Particles + velocity ramp | 2,587 | 31% |
| Particles + direction wheel | 2,708 | 32% |

Every look holds 120 Hz. Against `b8be8d4` every look without the
wheel sits inside the spread above, which is as much as this protocol
can say: the auto-ranging and the two running means are free, and
nothing here is a measurement of them being free to a hundred
microseconds.

**A branch costs what it contains.** The wheel's arithmetic went into
`disc_frag` and into the fill's flat branch, and an entry point's
registers are allocated for the whole of it, taken branch or not. Both
now have a second entry point; the fill's `flat_look` takes `wheel` as
a literal from each, so its branch folds at compile time. Two pairs,
each build installed straight after the other on the same still desk,
twice through:

| Pair | Wheel on a branch | Wheel in its own pipeline |
|---|---|---|
| Flat surface, one colour | 3,847 | 3,644 |
| Particle view, one colour | 2,592 | 2,578 |
| Particle view, wheel on | 3,140 | 3,021 |

The flat surface is where it bites, on the look that never enters the
branch: its fill returns in a dozen lines, so the shader's own cost is
most of what that look pays, where the glass runs the whole optics
after it. On the particle view the cost lands on the wheel's own draw
instead and the ordinary one is untouched.

An earlier reading put the particle view's share at 500 microseconds.
That was two sweeps an hour apart, not a pair, and it is withdrawn —
it is the reason the third protocol rule above exists.

This is the same shape as the varying finding above and it wants the
same habit: on this GPU a draw is priced by what its entry point *can*
do, not by what it does.

**A ramp on the flat surface is not free**, and never was:
147 microseconds, which is the fill's second texture sample
per water pixel and the splat's second channel. The earlier evening
measured the ramp on the particle view (free) and on the glass (76
microseconds) but never on the flat surface with the ramp on.

**The wheel is the expensive lens**, on the flat surface. On the
particle view it costs 121 microseconds over an ordinary ramp, and it
very nearly did not: sRGB's exact curve in the same fragment cost 613
of its own (3,589 against 2,976), five times what the whole lens costs
now and the largest single saving of the evening.

On the flat surface the wheel costs 1,235, and that is the second
field's decay and splat, which the particle view does not run. The
`pow` made no difference there, because the water covers fewer pixels
than the discs do.

The lever on the flat one is the flow field's resolution. It is a
quarter of the drawable in each dimension, like the thickness field,
and headings are far smoother than a water edge: an eighth would cut
its splat and decay fragments fourfold. That is a change to how the
look looks, so it is Jack's call, not a free win.

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
- `a_finger_drags_both_sides_of_the_water_it_crosses`: a settled pool,
  a finger through the middle at 1 m/s for an eighth of a second.
  Both sides of the stroke read positive x velocity — a repelling
  finger would throw its left side left. Measured this machine,
  2026-09-02: left 0.32 m/s, right 0.38 m/s, under the finger 0.35,
  beyond it -0.01. The last number is the return flow around the
  finger, and it pins the net's edge.
- `two_fingers_drag_their_own_water_their_own_way`: two fingers a
  radius and a fifth apart, pulling opposite ways. Measured this
  machine, 2026-09-02: the left neighbourhood -0.12 m/s, the right
  +0.11. A third of what one finger alone manages, because two fingers
  pulling apart are opening a gap in an incompressible fluid and the
  pressure solve stops them. The signs and the gap between them are
  the claim.
- `the_fingers_map_drawable_points_into_the_box` and
  `a_lifted_finger_leaves_the_others_where_they_were`: the corner
  mapping with its y flip, a lifted finger dropping out of the count,
  the smoothed speed against a known stroke, and a walking finger
  keeping its speed when the finger ahead of it in the packed block
  lifts.
- `step_immediates_land_at_the_shader_offsets`: the radius and the
  count at bytes 48 and 52, and two fingers at bytes 64 and 80.
- `DRAG=1` in the film harness: one finger, three settled seconds, a
  swipe across the pool, half a second held still, the swipe back, a
  lift. Filmed 2026-09-02: the water heaps ahead of the finger, a
  trough opens behind it, and the churn carries on after the lift.
  The harness drives slot 0 only; the two-finger case is the GPU
  test's.
- `a_ramp_paints_the_discs_between_its_two_colours` and
  `a_ramp_paints_the_flat_surface_between_its_two_colours`: red to
  blue across proximity, down each of the two paths. Every drawn pixel
  must sit on the line between the two colours, and the picture must
  not be one colour. Measured this machine, 2026-09-02: 59 distinct
  steps on the discs, 84 on the surface, both up from 53 and 47 when
  the ramp was derived — the frame's own ends are tighter, so the same
  water spends more of the ramp.
- `a_settled_pool_holds_its_colours_still`: how far a settled
  particle walks along its ramp between two frames, one number a lens.
  It replaces `a_settled_column_lands_inside_every_lens`, which
  checked where settled water fell on a *derived* ramp and went
  vacuous when the ramp became the frame's own two ends. The shimmer
  table above is its output. It also keeps two of the old test's
  claims, which the auto ramp does not make true by construction: a
  settled pool reads under a tenth of the speed ramp, which is the
  span floor's own test, and proximity separates the body from the
  free surface by a quarter of its ramp.
- `the_ramp_opens_faster_than_it_closes`: the chase, which runs on
  every frame the device draws and which no GPU test reaches, because
  each builds a `Ramp` of its own and the first ends are taken whole.
  It pins the three things the chase must do: hold off until the
  field has been read back, hold a zero span open at the floor, and
  cover half the step opening out against a sixth closing in.
- `the_wheel_paints_pure_hues_around_the_circle` and
  `the_wheel_paints_the_flat_surface_around_the_circle`: the wheel
  down each path, on a pool tipped hard on its side. Every drawn pixel
  must be a pure hue scaled by its speed, which is false the moment
  the clip-z pack leaks its hue into its saturation, and the water
  must land round the wheel rather than in a corner of it. Measured
  this machine, 2026-09-02: six sextants on the discs, five on the
  surface. Neither claim is made by the stability test above, and a
  broken pack or a missing heading field draws colour either way. Each
  now covers a pipeline of its own, since the wheel was split out of
  the disc draw and the fill.
- Every shell path is exercised by the menu; the shell has no test
  target. `FLUID_LOOK` names the look for a console run, which is how
  the measurements below reach a look the menu is not left in.
