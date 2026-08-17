# Web-export renderer spike

Phase-0 derisk for `plans/web-export.md` (2026-08-17): egui + odm-render on
WebGPU in a real browser. Throwaway code, kept as reference until the web
runtime lands; results are folded into the plan.

## What it proves (all green, Chromium 151 / Vulkan on RADV)

- **eframe's web backend + `Renderer::with_device` just works.** The
  spike forces `Backends::BROWSER_WEBGPU` (no fallback), renders into an
  offscreen texture (copy of viewer/viewport.rs) and paints it as an egui
  image — the exact viewer viewport structure.
- **The full pass structure validates on a browser WebGPU device**:
  opaque pass, depth-peeled translucency (overlapping translucent boxes
  layer correctly), unsorted tail, Rgba16Float blending, the line-quad
  wire/grid path with distance fade, overlay segments, and supersample
  (1×/2×/4×) — no validation errors, no visual artifacts.
- **Bitmap fonts are crisp** with `pixels_per_point` forced to an integer
  (same rule as native; the spike rounds the device pixel ratio).
- Size: 8.0 MB wasm at `opt-level = "s"` before any size pass
  (egui + wgpu + odm-render + embedded fonts).

## Running

```sh
cargo build --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir pkg \
    target/wasm32-unknown-unknown/release/web_hello_spike.wasm
python -m http.server 8741   # then open http://127.0.0.1:8741/

# headless screenshots (?wireframe / ?xray / ?ss=4 select a config):
chromium --headless=new --no-sandbox --enable-unsafe-webgpu \
    --enable-features=Vulkan --use-angle=vulkan --window-size=1280,800 \
    --virtual-time-budget=15000 --screenshot=out.png \
    'http://127.0.0.1:8741/index.html?ss=4'
```

No Manifold and no exotic toolchain here — plain wasm32-unknown-unknown +
`wasm-bindgen-cli` 0.2.126 (must match the Cargo.lock pin).
