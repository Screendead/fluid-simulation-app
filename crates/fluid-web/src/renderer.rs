//! The page's handle to the renderer. `requestAnimationFrame` drives
//! [`WebRenderer::frame`]; the page cancels the loop when hidden.

use crate::sample_from_device_motion;
use fluid_core::Renderer;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WebRenderer {
    inner: Renderer,
}

/// Builds a renderer on the canvas, sized by the canvas's width and height
/// attributes, which the page sets in device pixels. Rejects when the
/// browser withholds WebGPU.
#[wasm_bindgen]
pub async fn create_renderer(
    canvas: wgpu::web_sys::HtmlCanvasElement,
) -> Result<WebRenderer, JsValue> {
    let (width, height) = (canvas.width(), canvas.height());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let inner = Renderer::new(instance, surface, width, height)
        .await
        .map_err(|e| JsValue::from_str(&e))?;
    Ok(WebRenderer { inner })
}

#[wasm_bindgen]
impl WebRenderer {
    /// `now_ms` is the rAF timestamp; the six floats are the page's latest
    /// `DeviceMotionEvent` vectors in metres per second squared, zeros when
    /// the device has no sensors.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(&mut self, now_ms: f64, gx: f32, gy: f32, gz: f32, ax: f32, ay: f32, az: f32) {
        let sample = sample_from_device_motion([gx, gy, gz], [ax, ay, az]);
        self.inner.frame(sample, now_ms);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.inner.resize(width, height);
    }

    /// Off the frame path; the page calls this about once per second.
    pub fn stats_line(&self) -> String {
        let s = self.inner.stats();
        format!(
            "frames {} | frame interval µs p50 {:.0} p99 {:.0} max {:.0} | gpu µs p50 {:.0} p99 {:.0}",
            s.frames,
            s.interval_p50_us,
            s.interval_p99_us,
            s.interval_max_us,
            s.gpu_p50_us,
            s.gpu_p99_us
        )
    }
}
