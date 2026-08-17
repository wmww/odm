// Wasm side of the boolean timing. Run: node bench.mjs
import fs from 'node:fs';

const bytes = fs.readFileSync(
  new URL('./target/wasm32-unknown-unknown/release/web_fit_spike.wasm', import.meta.url));
const { instance } = await WebAssembly.instantiate(bytes, {
  env: { js_run_build: () => 0 },
});
const exp = instance.exports;
exp.odm_init();

for (const segments of [64, 128, 256]) {
  exp.bench_boolean(segments); // warm
  const RUNS = 3;
  let tris = 0;
  const start = process.hrtime.bigint();
  for (let i = 0; i < RUNS; i++) tris = exp.bench_boolean(segments);
  const perMs = Number(process.hrtime.bigint() - start) / 1e6 / RUNS;
  console.log(`segments ${segments}: ${tris} tris, ${perMs.toFixed(1)}ms/op`);
}
