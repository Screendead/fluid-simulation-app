//! Films a scripted 13-second trajectory to raw RGBA on stdout-adjacent
//! disk; scripts/film.sh turns it into an mp4. The poses mirror the ones
//! Jack's recordings cover: upright, the 45-degree corner, flat.

use std::fs::File;
use std::io::{BufWriter, Write};

const G: f32 = fluid_core::STANDARD_GRAVITY;

fn force_at(frame: u32) -> [f32; 3] {
    let t = frame as f32 / 120.0;
    // SHAKE=1: two violent upright seconds, then a settle. The splash
    // oracle.
    if std::env::var("SHAKE").as_deref() == Ok("1") {
        let mut f = [0.0, -G, -0.5];
        if (1.0..3.0).contains(&t) {
            f[0] += 30.0 * (t * 2.0 * std::f32::consts::PI * 4.5).sin();
            f[1] += 12.0 * (t * 2.0 * std::f32::consts::PI * 9.1).sin();
        }
        return f;
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
    f[0] += tremor * 0.20 * (t * 2.0 * std::f32::consts::PI * 6.3).sin();
    f[1] += tremor * 0.12 * (t * 2.0 * std::f32::consts::PI * 1.7).sin();
    f
}

fn main() {
    let dir = std::path::Path::new("target/film");
    std::fs::create_dir_all(dir).expect("mkdir");
    let mut raw = BufWriter::new(File::create(dir.join("film.raw")).expect("create"));
    let spacing: f32 = std::env::var("SPACING")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0025);
    let cap: u32 = std::env::var("CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    let dims = fluid_core::film(19 * 120, 4, spacing, cap, force_at, |rows| {
        raw.write_all(rows).expect("write");
    })
    .expect("no GPU adapter");
    raw.flush().expect("flush");
    println!("{}x{}", dims[0], dims[1]);
}
