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
  lands.
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
- **Absorption.** Beer-Lambert `exp(-sigma * t)` per channel, sigma
  chosen so blue survives: the thin edge reads as clear glass, the
  deep interior reads as water.
- **Light.** One directional light pinned to world-up by the gravity
  vector the shell already delivers every frame, plus a soft
  procedural gradient environment for the reflection term. The
  highlight slides with real tilt. No new sensor path; the sensor
  rule (CLAUDE.md section 7) is untouched.
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
  first calibrated films.
- The existing dance, ring, tilt and wake meters guard the physics:
  the renderer reads sim state and writes none.
- The M4 oracle (roadmap): looks like water; better than real time;
  inside budget — judged on the reference device, by Jack's eye.

## Budget

The composite rewrite is estimated at +0.3 to +0.8 ms GPU. That is an
estimate, not a measurement; the oracle is the before/after on the
reference device, and the measured number replaces this line when it
lands. For reference: settled 4x GPU p50 was 6.27 ms against the
8.33 ms frame (reference device, 2026-08-31), and motion runs higher.

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
