# Web-export fit spike

Phase-0 derisk for `plans/web-export.md` (2026-08-17): how Manifold, project
JS, and the Rust core plug together in a browser. Throwaway code, kept as
reference; results are folded into the plan and `notes/spike-findings.md`.

## What it proves

- **One wasm module, no emscripten.** `manifold-csg-sys` has an
  `unstable-wasm-uu` feature that builds Manifold + Clipper2 for
  wasm32-unknown-unknown with plain clang + wasm-ld against
  [wasm-cxx-shim](https://github.com/zmerlynn/wasm-cxx-shim)
  (no exceptions, no threads, no filesystem). odm-kernel + odm-store +
  odm-ir + jsonschema all link into the same module. Release wasm:
  **497 KB** total.
- **The ops seam works sync + reentrant.** JS host → `scheduler_build`
  (wasm) → imported `js_run_build` (JS "doohickey") → op exports (wasm) —
  the native `run_build` control flow with V8 replaced by the page realm.
  Mesh positions cross as a `Float64Array` view of wasm memory.
- **The real framework JS ships identical.** `frame.mjs` imports
  `framework/versions/unstable.js` + `framework/runtime/determinism.js`
  unmodified, supplies a `Deno.core.ops`-shaped glue backed by the wasm
  module, and runs a factory-wrapped doohickey through `__odm.runBuild`:
  CSG, volume query, console capture via op_log, frozen `Date`, IR JSON
  out, and a byte-identical second build from a fresh factory invocation.
- **Native and wasm Manifold agreed bit-exactly** on the probe (subtract
  volume bits `0x401c0031e9de621a` both sides, sin/cos exercised via
  sphere segments) — evidence the exported native store/memo snapshot can
  stay hash-consistent with browser rebuilds. One sample, not a proof.
- **Perf** (this machine, release, non-parallel Manifold both sides):
  sphere-subtract at segments 64/128/256 → 4.1/12.3/48.7 ms native vs
  5.3/16.9/63.8 ms wasm-in-node (~1.3× slower).

## Running

```sh
# toolchain: clang + wasm-ld + libc++ headers. Root-less setup used here:
export WASM_CXX_SHIM_LIBCXX_HEADERS=~/.local/opt/wasm-cxx/libcxx-headers
export WASM_CXX_SHIM_WASM_LD=~/.local/opt/wasm-cxx/wasm-ld
# (or: pacman -S libc++ lld and skip both)

cargo build --target wasm32-unknown-unknown --release
node run.mjs     # ops seam + reentrancy + mesh view
node frame.mjs   # real framework JS on wasm ops
node bench.mjs   # boolean timing (wasm side)
cargo run --release --bin native_bench   # native side + determinism probe
```
