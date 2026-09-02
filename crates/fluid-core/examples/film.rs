//! Films a scripted 13-second trajectory to raw RGBA on stdout-adjacent
//! disk; scripts/film.sh turns it into an mp4. The poses mirror the ones
//! Jack's recordings cover: upright, the 45-degree corner, flat. DRAG=1
//! holds the phone upright and strokes a finger across it instead.

use std::fs::File;
use std::io::{BufWriter, Write};

const G: f32 = fluid_core::STANDARD_GRAVITY;

// xorshift, so the noise is reproducible run to run.
fn noise(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state as f32 / u32::MAX as f32 - 0.5) * 3.464
}

// The box's angular velocity, rad/s about the screen axes. Only the
// SPIN pose rotates: ramp to one rev/s over half a second, hold two,
// stop - the hold after the stop is the swirl decay window.
fn omega_at(frame: u32) -> [f32; 3] {
    if std::env::var("SPIN").as_deref() != Ok("1") {
        return [0.0; 3];
    }
    let preroll: f32 = std::env::var("PREROLL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let t = frame as f32 / 120.0 - preroll;
    let w = match t {
        t if t < 3.0 => 0.0,
        t if t < 3.5 => 6.0 * (t - 3.0) / 0.5,
        t if t < 5.5 => 6.0,
        t if t < 6.0 => 6.0 * (6.0 - t) / 0.5,
        _ => 0.0,
    };
    [0.0, 0.0, w]
}

// Where the finger presses, normalised over the drawable, x right and
// y down. Only the DRAG pose touches: three settled seconds, a swipe
// across the pool at a hand's speed, half a second held still, the
// swipe back, then a lift. The entrainment oracle — the water must
// follow the finger and not flee it, brake under a still finger, and
// keep moving after the lift.
fn touch_at(frame: u32) -> Option<[f32; 2]> {
    if std::env::var("DRAG").as_deref() != Ok("1") {
        return None;
    }
    let preroll: f32 = std::env::var("PREROLL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let t = frame as f32 / 120.0 - preroll;
    let x = match t {
        t if t < 3.0 => return None,
        t if t < 3.5 => 0.15 + 1.4 * (t - 3.0),
        t if t < 4.0 => 0.85,
        t if t < 4.5 => 0.85 - 1.4 * (t - 4.0),
        _ => return None,
    };
    Some([x, 0.9])
}

fn force_at(frame: u32) -> [f32; 3] {
    // PREROLL: shift every pose schedule later by this many seconds,
    // so a scaled world's longer fall-and-settle finishes first. Every
    // pose clamps negative time to its initial state.
    let preroll: f32 = std::env::var("PREROLL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let t = frame as f32 / 120.0 - preroll;
    // NOISE: accelerometer noise in m/s^2 RMS per axis, the term the
    // harness never modelled and the phone always has. Measured on the
    // reference device 2026-08-31 (motion-log build, desk-still):
    // raw sigma 0.02-0.08 per axis, worst on z. The old standard 0.15
    // came from a handled-launch reading and overstates the still
    // phone about twofold; desk-still films run NOISE=0.08 (M3
    // record, "The noise, found").
    let sigma: f32 = std::env::var("NOISE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let mut rng = frame.wrapping_mul(2654435761).wrapping_add(1);
    let mut jitter = [0.0f32; 3];
    for j in &mut jitter {
        *j = sigma * noise(&mut rng);
    }
    // SHAKE=1: two violent upright seconds, then a settle. The splash
    // oracle.
    if std::env::var("SHAKE").as_deref() == Ok("1") {
        let mut f = [jitter[0], -G + jitter[1], -0.5 + jitter[2]];
        if (1.0..3.0).contains(&t) {
            f[0] += 30.0 * (t * 2.0 * std::f32::consts::PI * 4.5).sin();
            f[1] += 12.0 * (t * 2.0 * std::f32::consts::PI * 9.1).sin();
        }
        return f;
    }
    // SPIN: flat on the table, spun about the screen normal. Gravity
    // never moves in the box frame and userAcceleration stays zero, so
    // the gyro path is the only thing that can move the water: the
    // discriminating film for the rotation physics, and the swirl
    // meter's driver.
    if std::env::var("SPIN").as_deref() == Ok("1") {
        return [jitter[0], jitter[1], -G + jitter[2]];
    }
    // RECLINE: on its back, lifted ~17 degrees from flat — the pose
    // whose collective jumps Jack reported. FLAT: dead flat.
    if std::env::var("RECLINE").as_deref() == Ok("1") {
        return [jitter[0], -0.3 * G + jitter[1], -0.954 * G + jitter[2]];
    }
    if std::env::var("FLAT").as_deref() == Ok("1") {
        return [jitter[0], jitter[1], -G + jitter[2]];
    }
    // TILT: on its back at ~5 degrees, held four seconds, then the
    // tilt direction swings 180 degrees over eight. Jack's 2026-08-31
    // recording: the puddle must creep with the swing, not sit pinned
    // as a solid patch.
    if std::env::var("TILT").as_deref() == Ok("1") {
        let a = 0.09 * G;
        let phi =
            0.25 * std::f32::consts::PI + std::f32::consts::PI * ((t - 4.0) / 8.0).clamp(0.0, 1.0);
        return [
            a * phi.cos() + jitter[0],
            a * phi.sin() + jitter[1],
            -0.996 * G + jitter[2],
        ];
    }
    // RING: upright, three settled seconds, a quarter-second sideways
    // nudge, then still. The slosh ring-down oracle: water oscillates
    // near the box's gravity-wave mode (1.47 Hz at the shipped 4x
    // scale, 3.5-4 Hz at 1x) for seconds; jelly dies in one swing.
    if std::env::var("RING").as_deref() == Ok("1") {
        let mut f = [jitter[0], -G + jitter[1], -0.5 + jitter[2]];
        if (3.0..3.25).contains(&t) {
            f[0] += 1.0;
        }
        return f;
    }
    // DRAG: held upright and still, so the finger in touch_at is the
    // only thing that moves the water.
    if std::env::var("DRAG").as_deref() == Ok("1") {
        return [jitter[0], -G + jitter[1], -0.5 + jitter[2]];
    }
    // WAKE=1: six flat seconds — long enough to sleep — then a slow
    // eased tilt to recline. The idle-gate wake oracle: the gate must
    // sleep once, wake within the tilt's first two degrees, and never
    // freeze moving water.
    if std::env::var("WAKE").as_deref() == Ok("1") {
        let p = ((t - 6.0) / 2.0).clamp(0.0, 1.0);
        let e = 0.5 - 0.5 * (p * std::f32::consts::PI).cos();
        return [
            jitter[0],
            -0.3 * G * e + jitter[1],
            -G * (1.0 - 0.046 * e) + jitter[2],
        ];
    }
    let pose = |a: [f32; 3], b: [f32; 3], p: f32| -> [f32; 3] {
        let e = 0.5 - 0.5 * (p.clamp(0.0, 1.0) * std::f32::consts::PI).cos();
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * e)
    };
    let upright = [0.0, -G, -0.5];
    let corner = [-G * 0.707, -G * 0.707, -0.5];
    let flat = [0.0, 0.0, -G];
    let mut f = match t {
        t if t < 5.0 => upright,
        t if t < 7.0 => pose(upright, corner, (t - 5.0) / 2.0),
        t if t < 10.0 => corner,
        t if t < 12.0 => pose(corner, flat, (t - 10.0) / 2.0),
        t if t < 14.0 => flat,
        t if t < 16.0 => pose(flat, upright, (t - 14.0) / 2.0),
        _ => upright,
    };
    // A held phone is never still: two incommensurate tones at the
    // ~0.2 m/s^2 the telemetry shows in a steady hold. TREMOR=0 films
    // a tripod.
    let tremor: f32 = std::env::var("TREMOR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    f[0] += tremor * 0.20 * (t * 2.0 * std::f32::consts::PI * 6.3).sin() + jitter[0];
    f[1] += tremor * 0.12 * (t * 2.0 * std::f32::consts::PI * 1.7).sin() + jitter[1];
    f[2] += jitter[2];
    f
}

fn main() {
    let dir = std::path::Path::new("target/film");
    std::fs::create_dir_all(dir).expect("mkdir");
    let mut raw = BufWriter::new(File::create(dir.join("film.raw")).expect("create"));
    let spacing: f32 = std::env::var("SPACING")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fluid_core::WORLD_SCALE * 0.0025);
    let cap: u32 = std::env::var("CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    // The default length grows by the preroll so a shifted schedule
    // still fits; an explicit FRAMES is absolute.
    let preroll: f32 = std::env::var("PREROLL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let frames: u32 = std::env::var("FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(((19.0 + preroll) * 120.0) as u32);
    let dims = fluid_core::film(
        frames,
        4,
        spacing,
        cap,
        |f| (force_at(f), omega_at(f), touch_at(f)),
        |rows| {
            raw.write_all(rows).expect("write");
        },
    )
    .expect("no GPU adapter");
    raw.flush().expect("flush");
    println!("{}x{}", dims[0], dims[1]);
}
