import init, { body_force, create_renderer } from "./pkg/fluid_web.js";

await init();

const canvas = document.getElementById("surface");
const readout = document.getElementById("readout");
const stats = document.getElementById("stats");

// The latest DeviceMotionEvent vectors, m/s²; zeros until sensors speak.
const latest = { gx: 0, gy: 0, gz: 0, ax: 0, ay: 0, az: 0 };
let samples = 0;

function fitCanvas() {
  canvas.width = Math.round(canvas.clientWidth * devicePixelRatio);
  canvas.height = Math.round(canvas.clientHeight * devicePixelRatio);
}

let renderer = null;
if (!("gpu" in navigator)) {
  stats.textContent = "this browser has no WebGPU; readout only";
} else {
  fitCanvas();
  try {
    renderer = await create_renderer(canvas);
  } catch (e) {
    stats.textContent = `no renderer: ${e}`;
  }
}

let rafId = null;
function frame(nowMs) {
  renderer.frame(nowMs, latest.gx, latest.gy, latest.gz, latest.ax, latest.ay, latest.az);
  rafId = requestAnimationFrame(frame);
}

if (renderer) {
  rafId = requestAnimationFrame(frame);
  setInterval(() => {
    if (!document.hidden) stats.textContent = renderer.stats_line();
  }, 1000);

  // Idle costs nothing: a hidden page runs no frame.
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      cancelAnimationFrame(rafId);
      rafId = null;
    } else if (rafId === null) {
      rafId = requestAnimationFrame(frame);
    }
  });

  window.addEventListener("resize", () => {
    fitCanvas();
    renderer.resize(canvas.width, canvas.height);
  });
}

function show(event) {
  const g = event.accelerationIncludingGravity;
  const a = event.acceleration;
  // A browser without sensor data still fires the event, with null vectors.
  if (g?.x == null || a?.x == null) return;
  Object.assign(latest, { gx: g.x, gy: g.y, gz: g.z, ax: a.x, ay: a.y, az: a.z });
  const f = body_force(g.x, g.y, g.z, a.x, a.y, a.z);
  samples += 1;
  readout.textContent =
    `including gravity (m/s²)  ${fmt(g.x)} ${fmt(g.y)} ${fmt(g.z)}\n` +
    `acceleration (m/s²)       ${fmt(a.x)} ${fmt(a.y)} ${fmt(a.z)}\n` +
    `body force (m/s²)         ${fmt(f[0])} ${fmt(f[1])} ${fmt(f[2])}\n` +
    `samples ${samples}`;
}

const fmt = (v) => (v >= 0 ? "+" : "") + v.toFixed(2).padStart(6);

// iOS Safari grants DeviceMotion only from a user gesture on a secure origin.
document.getElementById("enable").addEventListener("click", async () => {
  if (!window.isSecureContext || !("DeviceMotionEvent" in window)) {
    readout.textContent = "motion needs a secure origin (https or localhost) and a browser with sensors";
    return;
  }
  if (typeof DeviceMotionEvent.requestPermission === "function") {
    const state = await DeviceMotionEvent.requestPermission();
    if (state !== "granted") {
      readout.textContent = `motion permission ${state}`;
      return;
    }
  }
  window.addEventListener("devicemotion", show);
  readout.textContent = "listening; no sample yet";
  setTimeout(() => {
    if (samples === 0) {
      readout.textContent = "no motion samples: this device has no motion sensors, or the browser withholds them; open the page on a phone";
    }
  }, 2000);
});
