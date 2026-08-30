# M2 — The particles

*Design record, 2026-08-30. Binds the code with `decisions.md`.*

## 1. Goal

Tilt the phone and tens of thousands of particles slosh. A GPU particle
buffer, integration under the body force, box collision, point rendering.
The first WGSL shader; the `wgsl` feature enters (D4 pre-records it).
No inter-particle forces: that is M3.

## 2. Decisions

- **2D, in the screen plane.** Positions and velocities are `vec2` in the
  device frame's x and y; the body force's z component is unused until
  M3 brings a real box. A face-up phone therefore has near-zero in-plane
  force and the particles float — a behaviour, not a bug; held at any
  tilt they rain to the low edge.
- **Real scale.** The box is the visible screen at physical size:
  metres from pixels at 458 ppi, the reference device's density, a core
  constant until a second device matters. Gravity then feels real: a
  drop crosses the screen's height in about a sixth of a second.
- **One storage buffer**, position and velocity interleaved, 16 bytes a
  particle. Integration is in place; nothing reads a neighbour, so there
  is no ping-pong. One uniform buffer carries force, dt, extents, count
  and sprite radius.
- **Deterministic seeding.** A lattice filling the upper half of the box,
  jittered by an inline index hash. No RNG dependency; launch is the
  demo — the block collapses.
- **Hash-decohered dynamics.** Observed on the reference device,
  2026-08-30: with no inter-particle forces, identical dynamics collapse
  every particle onto one point — same force, same drag, same wall clamp —
  and the demo becomes a single dot. Until M3 brings real volume
  exclusion, each particle gets a hash-derived personal wall inset
  (squared, so density peaks at the wall), restitution, and a slightly
  tilted, scaled force. No extra memory; a few ALU ops in the compute
  shader. Rejected: a neighbour grid (that is M3), per-frame jitter alone
  (a shimmering dot, still one point). Second observation, Jack,
  2026-08-30: per-axis insets proportional to the extents settle into a
  screen-aspect rectangle. The offsets now sample a quarter disc in
  physical units, densest at the wall, so a settled cluster is a rounded
  pool. Third observation, Jack's screenshot, 2026-08-30: a fixed force
  rotation leaks particles up a wall the force runs nearly parallel to —
  a subset gains a net upward component and settles in the wrong corner.
  The force now varies in magnitude only (0.9 to 1.1).
- **Idle is deferred, explicitly.** CLAUDE.md section 7 says a still
  phone runs no simulation step. M2's placeholder integrates every
  foregrounded tick; backgrounding pauses the display link. Stillness
  sleep arrives with M6, as the roadmap has always planned; this sentence
  is the explicit deferral, on the record for Jack.
- **Compute then draw, one encoder.** Integrate, damp, reflect off the
  four walls with restitution. Render as instanced four-vertex strips,
  vertex-pulled from the particle buffer: soft circular sprites, additive
  blend, colour from speed. The background stays the body-force tint,
  scaled toward black so the sprites carry the scene.
- **Runtime knobs for the ramp.** Particle count and sprite radius enter
  through `fluid_renderer_create`; the shell reads `FLUID_PARTICLES` and
  `FLUID_RADIUS` from the environment so a `devicectl` launch can sweep
  configurations without a rebuild. Defaults live in the shell.

## 3. Budget protocol

The adapter offers no GPU timestamps (M1 record, section 4), so sustained
cadence is the GPU meter. Overdraw in the settled pile is the cost that
breaks first — sprite radius, not count, is the killer knob. Ramp count
and radius on the device; the largest configuration that holds interval
p99 = 8,334 µs over a minute is the M2 oracle number, recorded in
HANDOFF with device and date.

## 4. Tested and exercised

The seeding function is pure and tested: count, bounds, upper-half
placement. The shaders are untested and exercised: the simulator run and
the reference-device runs of 2026-08-30, Jack's hand among them. Their
logic is kept minimal for that reason. The M1 stats machinery is reused
unchanged. The shell's stats call moved off the frame path (it ran every
tick in M1 — a bug against the M1 record's own line). Measured on the
reference device, 2026-08-30, Release: 102 µs a call, now paid once per
120 ticks instead of every tick — about 101 µs a frame saved.

## 5. Exit

- [x] Shader validates in the simulator leg (2026-08-30).
- [x] Particles rain, slosh and pool on the phone under Jack's hand
      (2026-08-30; three artifacts observed and fixed, section 2).
- [ ] Ramp run — cut 2026-08-30, Jack's call: progress straight to M3.
      A budget ramp of a placeholder loop M3 replaces has no value; M3
      carries the budget work. Hand-test frame numbers are in HANDOFF.
- [x] Gate green at every commit; CI green on the branch.
