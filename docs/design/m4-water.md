# M4 — Water: the liquid-glass renderer

*Design record for M4. Binds the code. Amend by explicit edit.*

## The directive

Jack, 2026-08-30, verbatim: "the various different lenses (e.g.
velocity, density, acceleration, temperature, pressure, etc) should be
behind a dropdown menu. the default should be as photorealistic of a
water renderer as possible while being better-than-realtime rendering."

Jack, 2026-08-31, verbatim: "How realistic is an Apple-style 'Liquid
Glass' renderer? (look it up if you are unfamiliar) maybe with some
black and white navy-dazzle-pattern stripes behind it?" On the
assessment, verbatim: "Nice, lock it in."

The direction: the default view renders the water as liquid glass —
real refraction, Fresnel reflectance, a specular that tracks real
device motion — over a procedural black-and-white dazzle pattern on
the back wall of the box. The dazzle backdrop is set dressing, not a
physics compromise. The light transport is the photoreal part; a
high-contrast backdrop is what makes refraction visible at all. The
backdrop is Jack's aesthetic commitment, 2026-08-31.

## The optics model

The rendering model of the slab: a heightfield front surface over a
flat back wall, viewed orthographically. Within that model the
refraction is exact. The model itself approximates the sim — the
splatted field is a density proxy, and the true free surface is three
dimensional — so every "exact" claim stops at the model boundary.

- **Thickness.** The existing quarter-resolution `R16Float` field
  (splatted, EMA-smoothed by the keep/decay blend) maps to water
  thickness in metres. The map is a calibration step, not an assumed
  constant: measure the settled interior field value on a film, pin
  it to `SLAB_DEPTH`, and record the measured value here when it
  lands. Measured 2026-08-31: settled interior plateau 5.297 (raw
  splat after 600 upright frames at production scale, headless GPU).
  `FIELD_SETTLED = 5.3`; the calibration test re-measures on every
  gate run and holds the constant to ±10%.
- **Normal.** Per pixel, from the thickness gradient:
  `n = normalize(-s*dh/dx, -s*dh/dy, 1)`. The quarter resolution and
  the temporal EMA are the smoothing; add a small separable blur of
  the field only if the flicker meter demands it.
- **Refraction.** Refract the view ray at the front surface with
  eta = 1/1.33 (Snell, water), carry the refracted ray through the
  local thickness to the back-wall plane, sample the backdrop where
  it lands. Rejected: the screen-space UV-offset approximation
  (offset proportional to `n.xy` times a strength dial) — the same
  cost, and our geometry is the one case where the honest form is
  available.
- **Fresnel.** Schlick, F0 = 0.02 (water), blending the reflected
  environment against the refracted backdrop. The rim brightening is
  emergent at grazing angles; the hand-painted `RIM` constant in
  `sim_surface.wgsl` comes out.
- **Absorption.** Beer-Lambert `exp(-sigma * s)` per channel, with
  `s` the refracted path length through the water — at oblique
  incidence it exceeds the vertical thickness, as it should. Sigma
  chosen so blue survives: the thin edge reads as clear glass, the
  deep interior reads as water. Chosen 2026-08-31: transmittance
  (0.55, 0.78, 0.93) at one slab depth of path. Rejected:
  (0.35, 0.65, 0.85) — on film the body read as a flat blue overlay,
  not glass.
- **Caustics** (2026-09-01, Jack's pick from the visibility options).
  The refracted wall sample is scaled by the reciprocal Jacobian of
  the refraction map, linearised: `1 - (1-eta)(|grad t|^2 + t lap t)`,
  clamped to [0.5, 2.0]. It is the same map the sample itself takes,
  so brightness and stripe stretch stay mutually consistent —
  compressed stripes brighten, magnified ones darken; bright bands
  ride wave crests. `CAUSTIC = 0.35`; 1.0 and 0.6 rejected on film —
  the settled body glowed with dapple. The clamp floor is the
  glass-edge shadow at the waterline.
- **Frost** (2026-09-01, same pick). The stripe filter width widens
  by `FROST * path` — the wall reads milky through deep water and
  crisp through the thin edge, separating the body with no palette
  change. `FROST = 0.007` m of blur per slab depth of path.
- **Field blur.** The optics read the field through a 7-tap separable
  Gaussian (two quarter-res passes, field -> blur_a -> blur_b). This
  is the record's blur contingency, demanded by the flicker meter:
  caustics are a second-derivative effect, and the raw footprint
  ripple saturated the gain into full-body speckle (71,629 px/frame).
  The blur is a wavelength filter — particle-footprint ripple lives
  near the inter-particle spacing and dies; waves live at ten times
  that and pass. The splat/EMA path still writes the raw field; the
  calibration is untouched (the kernel sums to one).
- **Light.** One directional light pinned to world-up by the gravity
  vector the shell already delivers every frame, plus a soft
  procedural gradient environment for the reflection term. The
  highlight slides with real tilt. No new sensor path; the sensor
  rule (CLAUDE.md section 7) is untouched. One deliberate guard,
  2026-08-31: the sun fades to zero as world-up aligns with the view
  axis. An orthographic view under a directional light is degenerate
  on flat water — every pixel hits the glint angle at once — and a
  face-up pose is exactly that. Rejected: fade floors of 0.15 and
  0.05, which let the glint lobe resolve the one-layer particle
  lattice of a flat pool into a honeycomb of florets.
- **Backdrop.** Procedural dazzle: a small set of angular sectors,
  each with its own stripe direction and width, black and white only,
  filtered analytically against aliasing (smoothstep at filter width
  on the stripe function). Zero texture memory, crisp under
  refraction magnification. Stripe widths, angles and seed are Jack's
  aesthetic dials; defaults land with the first build. Rejected: a
  texture backdrop — memory, mip aliasing under refraction, a
  resolution ceiling.

## Meters and the oracle

- **Stripe-flicker meter** (new, film harness): temporal delta of the
  refracted backdrop in a settled pose, px/frame, in the family of
  the dance meter. Refraction amplifies normal noise, and the stripes
  make any residual edge shimmer visible — the same property that
  makes the look work makes it measurable. Threshold set from the
  first calibrated films. Calibrated 2026-08-31 (settled upright,
  NOISE=0.08, TREMOR=0, IDLE=0, production keep, delta > 12 grey
  steps): mean 8,787 px/frame — 1.0% of the frame — p95 19,127. The
  old flat-fill renderer measures 1,893 on the same films: the glass
  amplifies settled shimmer 4.6x, as predicted. The static wall
  measures 0-1, so the meter reads pure renderer. The churn is
  interior stripe-crawl from real particle drift under sensor-noise
  jitter; the glint band itself is quiet (0.46% of its rows), and
  the idle gate zeroes the whole number at true rest. Threshold:
  settled-awake mean <= 12,000 px/frame on this recipe.
  Recalibrated 2026-09-01 on the caustics build: mean 3,268 (raw
  caustics before the field blur: 71,629; the first glass build:
  8,787; the old flat fill: 1,893). The threshold stands.
- The existing dance, ring, tilt and wake meters guard the physics:
  the renderer reads sim state and writes none.
- The M4 oracle (roadmap): looks like water; better than real time;
  inside budget — judged on the reference device, by Jack's eye.

## Budget

Estimated +0.3 to +0.8 ms GPU before the build. Measured on the
reference device, 2026-08-31 night, thermal state "serious" on both
sides of every pair, settled upright, pre-sleep awake window:

| Build | GPU p50 | Note |
|---|---|---|
| old renderer (7416b49 line) | 6.694 ms | re-measured, same session |
| first glass build (426e9a8) | 8.563 ms | +1.87 ms; over the frame, drops to intermittent 60 Hz |
| restructured (5728036) | 7.557 ms | +0.86 ms; inside the 8.33 ms frame, 120 Hz held |
| lane-parallel solver + 4.2 ms cap (e54c456 line) | 6.15 ms | 2026-09-01 morning, thermal "fair"; below the pre-glass baseline (optimisation record) |

The restructure that bought the millisecond back: lighting that is
uniform across a frame (world up, glint half vector, folded gain)
precomputed into the immediates; per-sector stripe families baked to
a table; stripe filtering by the analytic screen-space rate instead
of fwidth, which frees air pixels to take an early return past the
whole water path. All three were confirmed findings of the
adversarial review of 426e9a8.

Motion, same session, from Jack's hard shake: the frame enters a
locked failure basin. Captured timeline: v decays to 0.01 — water at
rest — while substeps stay pinned at 8, the display at 60 Hz, GPU at
~19 ms, because 60 Hz makes dt 16.7 ms, dt floors substeps at 8 for
stability, and ~184 solver dispatches at 50-90 us fixed overhead each
cost more than the 16.7 ms frame: the loop feeds itself. The idle
gate's sleep is currently the only exit — the wake after sleep
returns at n 4, 8.3 ms, 120 Hz. This basin is the optimisation
pass's first target; the capture lives in the session scratchpad
(after2-m4.txt). Closed 2026-09-01: the basin is dead on the device
(optimisation record, "The device session").

## The first device session (2026-08-31, night)

Jack's verdict, verbatim: "this looks incredible. shook it, tilted
it, played for a while". The M4 oracle's "looks like water" clause
passes its first reading. Four notes, verbatim, with dispositions:

1. "minor jitter when upright and still" — the interior stripe-crawl
   the flicker meter already quantifies (settled-awake mean 6,700-
   8,800 px/frame; the idle gate zeroes it once asleep). Work item:
   the record's blur contingency — a small separable blur of the
   field before the gradient, judged by the flicker meter and the
   eye. Resolved 2026-09-01: the blur landed as the caustics'
   foundation, and the settled-awake flicker fell to 3,268 px/frame —
   half the pre-caustics build. See "Field blur" above.
2. "you can tell (from looking at the reflections) that the water
   inside the body isn't really 'swirling' much even after large
   movements - but it should be, ideally" — true by construction:
   the optics read the splatted thickness field only, so internal
   motion with a flat surface is invisible. Direction to design: an
   advected perturbation field — a quarter-res scalar advected
   through the existing velocity grid (the tracer machinery already
   advects through it), injected by speed, decaying over seconds,
   perturbing the refraction normal. Water would then visibly churn
   after motion and calm down. Superseded 2026-09-01: the dye field
   ("The dye, designed") is that idea made honest — the memory rides
   the tracers, and it modulates scatter, not the normal.
3. "the water is a little hard to see, when it's the main subject
   and we've put so much work into it. i'm not sure the right
   approach is to colour it differently - suggest some options" —
   options tabled for Jack's pick in HANDOFF; front-runners are
   caustics on the back wall (brightness from surface curvature —
   light focused by the wavy surface) and scattering (the wall reads
   slightly frosted through water: widen the stripe filter width
   with thickness — near-free, no palette change). Ruled 2026-09-01,
   Jack verbatim: "1+2, go for it" — caustics and frost, both landed;
   dials above.
4. "the performance is still questionable, given a hard shake it
   will then go back into the lag failure mode where subsequent
   movements are not reacted to in real time" — the substep-floor
   basin measured and mechanised in the Budget section above.
   Optimisation-pass target. Open.

## The dye, designed (2026-09-01)

Jack's direction, verbatim: "let's do the dye field". The problem it
solves is recorded in the M3 record ("The sink, measured"): a flat
surface hides internal motion from thickness-only optics, so the
retained swirl is invisible until it deforms the surface.

The model. Each tracer carries a charge: the fastest box-frame speed
it has recently felt, decaying with time constant T_DYE. The advect
kernel updates it (charge = max(|v|, charge * exp(-dt / T_DYE))) and
the charge rides the existing f16 speed slot — no format change. A
quarter-res R16Float dye texture accumulates the charge through the
field's own decay-plus-splat pattern (the motion-aware field_keep,
the same blend-constant template), so the texture carries no shot
noise at rest and clears itself in motion. Tracers splat as small
soft quads (the body/weight falloff form), not points: ~2.4 tracers
per dye texel is Poisson-noisy as single pixels. The surface shader
reads dye and mixes `through` toward a pale scatter tint before the
Fresnel blend — the milkiness lives inside the water, the
reflection stays clean.

Why this is honest, and not the withdrawn perturbation field: the
charge sources on box-frame relative speed from the solved field,
and the memory advects with the tracers through the solver's own
velocity grid. Steady rigid co-rotation makes no dye (no relative
motion, no aeration — correct); spin-up, the stop, eddies and slosh
all do. Swirls stretch and fold the charged tracers into filaments;
the structure is real advection, not synthesis.

Dials, Jack's. First-build values from the film ladder, 2026-09-01:
T_DYE 4 s, DYE_GAIN 0.18, MILK_MAX 0.35, DYE_FLOOR 0.09 m/s, DYE_R
0.01 m, DYE_SCALE 0.4, MILK_TINT (0.75, 0.82, 0.88). Rejected: gain
0.5, cap 0.6, floor 0.05 — the whole body fogged to a uniform grey
wash four seconds after the spin stop, because residual swirl above
0.05 m/s kept re-sourcing charge and the gain saturated the cap
everywhere; the dazzle died behind it. At the shipped dials the stop
blooms, the churn draws banded haze, and the glass is clear again
about four seconds after the swirl dies. Flicker on the meter recipe
(NOISE=0.08 TREMOR=0 IDLE=0, settled window): 4,459 px/frame with
dye against 3,525 without on the same build and day — both re-runs
of the recipe behind the recorded 3,268 — threshold unchanged at
12,000. Jack's device eye rules the final values.

Two recorded contingencies:
- Respawn erosion. Tracers respawn with time constant TAU = 3 s
  (calibrated against cloud collapse; not touched). At T_DYE = 4 s
  the filament contrast decays with tau_eff ~ 1.7 s while the eddy
  it traces lives ~5 s. If the films show filaments dying before the
  swirl, respawn inherits the previous frame's dye texture at the
  spawn point instead of the source particle's speed.
- The dot draw shares the speed slot, so dots now glow for ~T_DYE
  after motion stops. If Jack's eye reads that as the old speckle
  species, decoupling costs a tracer widening (vec2u -> vec4u,
  ~1 MB) and its own slot.

Budget. The splat's cost is overdraw: a 1 cm quad covers ~500 dye
texels on the device target, so 131,072 charged tracers rasterize
~66M blended fragments — far above the first +0.3-0.6 ms estimate
(the review caught the arithmetic). Mitigation, shipped: a quad with
zero charge collapses to a point in the vertex stage, so a settled
box pays nothing and the cost scales with how much water is
churning. Worst case (every tracer charged, a hard shake) is brief
by construction — the charge floor and T_DYE drain it — and lands
during frames the solver already spends heavily. The device p50,
settled and shaken, prices it before any merge; if the shaken number
is unacceptable, the recorded fallback is a stochastic fraction of
the tracers splatting at proportionally higher DYE_SCALE.

Priced 2026-09-01, reference device, thermal nominal, 88 one-second
stats windows of Jack's play: settled GPU p50 6.1 ms — unchanged
from the pre-dye 6.15, the zero-charge collapse holds. Post-shake
churn (the dye-heavy state) 6.3-7.8 ms; the single worst window
9.8 ms p50 during a v 3.2, n 16 shake, with drops only in the p99
tail as before the dye. Every window held the 120 Hz median — no
60 Hz window in the whole session. The stochastic fallback is not
needed.

## Sequencing: renderer before the aggressive optimisation pass

Decision, 2026-08-31. Build the M4 renderer first. Run the aggressive
optimisation pass once, after it, on the complete frame. Rejected:
optimisation first — it tunes a frame that is missing its main render
cost, and every measurement would be retaken after M4 lands.

Consequence for M3 close: the M3 oracle carries an "inside budget"
clause, so under the roadmap's closure rule M3 stays open until the
optimisation pass lands its numbers. Closing M3 earlier is Jack's
call, and takes an explicit amendment of that rule.

Jack asked on 2026-08-31 to investigate variable particle density and
sizing (adaptive resolution). That is an optimisation-pass technique;
the investigation's findings and the go/no-go land in the
optimisation-pass design, not here.
