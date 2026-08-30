import init, { body_force } from "./pkg/fluid_web.js";

await init();

const readout = document.getElementById("readout");
let samples = 0;

function show(event) {
  const g = event.accelerationIncludingGravity;
  const a = event.acceleration;
  // A browser without sensor data still fires the event, with null vectors.
  if (g?.x == null || a?.x == null) return;
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
