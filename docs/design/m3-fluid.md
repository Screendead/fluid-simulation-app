# M3 — The fluid

*Design record, 2026-08-30. Binds the code with `decisions.md`. The method
decision is D5.*

## 1. Goal

Real water in the box. Jack's directive (verbatim in HANDOFF, 2026-08-30):
a full fluid simulation, physically accurate, with calculated pressure,
density, velocity, acceleration and temperature, as close to real water as
possible at the true physical size of the reference device. The water-style
renderer is explicitly left out: that is M4. M3 renders with the M2
sprites and puts field statistics on screen.

## 2. What "1:1 with real water" means here

Real water is ~10^25 molecules in this box. Every real-time method solves
the continuum equations instead. M3 solves incompressible Navier–Stokes
with SPH at particle spacing d, with real SI constants and measured error.
The deviations from reality, each deliberate and bounded:

| Deviation | Bound |
|---|---|
| Continuum at spacing d (1.5–3 mm; stage 0 fixes it) | Sub-d eddies and droplets do not exist |
| Incompressibility to a target, not exactly | Density error target: 0.1% average, 1% max |
| CFL clamp on velocity per substep | Violations counted and shown on screen, never silent |
| Air is vacuum; one-phase fluid | No bubbles, no drag from air |
| Surface tension omitted in M3 | Millimetre droplets behave too heavily; revisit at M4 |

Constants, all SI at 20 °C: density 998.2 kg/m³, dynamic viscosity
1.002 mPa·s, gravity from the sensors (D3), heat capacity 4184 J/(kg·K),
thermal conductivity 0.598 W/(m·K).

The box: the visible screen at physical size (458 ppi, M2's constant) by
the device's 7.65 mm depth. The simulation is 3D in a thin slab — real
water in a slab moves in 3D, and M4's screen-space renderer needs depth.
2D is rejected in D5.

## 3. Temperature, honestly

Temperature is transported per particle and is physically real, and its
real signal is microkelvin. The arithmetic, so the lens design is a
decision and not an apology:

- Pressure work: ~0.018 K per MPa adiabatic; slosh pressures are kPa
  (hydrostatic floor ~1.5 kPa) → sub-µK, and the incompressibility solve
  drives the compression term toward zero anyway.
- Viscous dissipation: µ·(∇v)² over c_p — µK/s at slosh shear.
- Diffusion: SPH Laplacian with real conductivity.
- Thermal expansion feedback on density: β ≈ 2.07×10⁻⁴/K × µK ≈ 10⁻¹⁰
  relative — negligible by arithmetic, recorded here, not built.

The M5 temperature lens auto-scales its colour range to the live min–max,
so µK structure is visible without faking magnitudes.

## 4. The solver

DFSPH (D5), one frame:

1. Neighbour grid: counting sort by cell (cell = support radius), with a
   workgroup prefix scan written in WGSL. Validated in the simulator leg
   before anything builds on it.
2. Per substep, count from the CFL bound (dt ≤ 0.4·d/v_max, v_max from a
   GPU reduction — no readback on the frame path):
   density and DFSPH factor; divergence-free solve; semi-implicit Euler
   under the body force and Morris viscosity; constant-density solve;
   position update; temperature sources and diffusion.
3. Draw with the M2 sprite pass, colour by speed, plus on-screen field
   statistics: density error %, pressure min–max, temperature min–max,
   substep count, CFL-clamp count. Every field has a reader from day one.

Per-substep uniforms go through push constants (`immediate_size`) if the
device grants the feature — stage 0 verifies — else a params buffer with
per-substep dynamic offsets. `Queue::write_buffer` allocates a staging
buffer per call (wgpu 30 source) and does not belong in a substep loop.

## 5. Resolution and budget — measured, not asserted

CFL arithmetic at v_max = 2 m/s (hard shake): d = 0.33 mm → dt ≤ 66 µs →
~126 substeps per 8.33 ms frame — dead. d = 1.5 mm → dt ≤ 300 µs → ~28.
d = 2 mm → ~21. d = 3 mm → ~14. Counts in the slab (interior ≈ 154 × 71 ×
7.65 mm): d = 1.5 mm → ~25k particles; 2 mm → ~10k; 3 mm → ~3k.

Stage 0 is a microbenchmark on the reference device before the solver
exists: seed N, build the grid, run K density sweeps per frame; ramp N
and K; read the GPU timestamp span. Cost per sweep × substeps × solver
iterations then fixes d and the particle count arithmetically, and this
record gains the measured table before the solver stages begin.

## 6. Deferrals, explicit

- The water-style renderer: M4, by Jack's directive above.
- The lens dropdown UI: M5 (HANDOFF roadmap; Jack's directive of
  2026-08-30 on the default view binds it).
- Stillness sleep ("idle costs nothing"): M6, as in the M2 record.
- Rotation, Coriolis and Euler forces: M7 (O6). The box frame stays
  non-rotating in M3.

## 7. Tested and exercised

Pure and tested: the CFL arithmetic, kernel normalisation, the prefix
scan (against a CPU scan), grid cell indexing, the temperature source
terms. Exercised on device and simulator, untested: the WGSL solver
passes, kept minimal per pass. The on-screen field statistics are the
standing exercise of every computed field.

## 8. Exit

- [ ] Stage-0 microbench table in this record; d and count chosen from it.
- [ ] Prefix scan validated in the simulator against a CPU reference.
- [ ] At rest: density error inside target; pressure reads hydrostatic
      (~1.5 kPa floor); temperature drift bounded. Numbers in HANDOFF.
- [ ] Under Jack's hand: a convincing slosh, 120 Hz interval p99 within
      budget over a minute, measured and in HANDOFF.
- [ ] Gate and CI green.
