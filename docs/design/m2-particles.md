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
  (a shimmering dot, still one point).
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
placement. The shader is exercised on the device and simulator, untested;
its logic is kept minimal for that reason. The M1 stats machinery is
reused unchanged. The shell's stats call moves off the frame path (it ran
every tick in M1 — a bug against the M1 record's own line; fixed here and
measured for the ledger).

## 5. Exit

- [ ] Shader validates in the simulator leg (cheap loop for WGSL errors).
- [ ] Particles rain, slosh and splash on the phone under Jack's hand.
- [ ] Ramp run: count and radius at budget recorded in HANDOFF.
- [ ] Gate and CI green.
