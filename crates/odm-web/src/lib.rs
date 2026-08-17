//! Browser host for an exported ODM project: the same store + build engine +
//! kernel + renderer + viewer core as the desktop engine, with the page's
//! own JS as the executor (no V8-in-wasm). One frozen generation, one view,
//! synchronous builds on the main thread — see plans/web-export.md.
//!
//! Native builds get an empty stub: the wasm lane needs the exotic
//! clang/wasm-ld toolchain and is built only by `cargo xtask
//! build-web-template`.

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod executor;
#[cfg(target_arch = "wasm32")]
mod host;

#[cfg(target_arch = "wasm32")]
mod entry {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    /// Page entry, called by runtime.js after the bundle registry and the
    /// ops glue are in place: parse the manifest, build the engine around
    /// the frozen generation, start the egui app on the canvas.
    #[wasm_bindgen]
    pub async fn odm_web_start(manifest_json: String, canvas_id: String) -> Result<(), JsValue> {
        console_error_panic_hook::set_once();
        let manifest: crate::host::Manifest = serde_json::from_str(&manifest_json)
            .map_err(|e| JsValue::from_str(&format!("manifest.json: {e}")))?;
        let engine = crate::host::WebEngine::new(manifest);

        let document = web_sys::window()
            .and_then(|w| w.document())
            .ok_or_else(|| JsValue::from_str("no document"))?;
        let canvas = document
            .get_element_by_id(&canvas_id)
            .ok_or_else(|| JsValue::from_str("canvas element missing"))?
            .dyn_into::<web_sys::HtmlCanvasElement>()?;

        // WebGPU only — same renderer of record as native, no fallback path.
        let mut options = eframe::WebOptions::default();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) =
            &mut options.wgpu_options.wgpu_setup
        {
            setup.instance_descriptor.backends = odm_render::wgpu::Backends::BROWSER_WEBGPU;
        }

        eframe::WebRunner::new()
            .start(canvas, options, Box::new(|cc| Ok(Box::new(crate::app::WebApp::new(cc, engine)))))
            .await
    }
}
