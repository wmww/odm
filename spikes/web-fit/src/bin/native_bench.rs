//! Native side of the wasm-vs-native boolean timing. Same code path as the
//! wasm `bench_boolean` export (note: non-parallel Manifold here too, so the
//! comparison isolates the toolchain, not threading).
fn main() {
    web_fit_spike::odm_init();
    // Cross-toolchain determinism probe: same solid as run.mjs's reentrant
    // build; compare the volume's exact f64 bits against the wasm side.
    let box_h = web_fit_spike::op_solid_box(2.0, 2.0, 2.0);
    let ball = web_fit_spike::op_solid_sphere(1.25, 32);
    let root = web_fit_spike::op_boolean(1, box_h, ball, 1.0, 1.0, 1.0);
    let vol = web_fit_spike::op_volume(root);
    println!("probe: {} tris, volume bits 0x{:016x} ({vol})", web_fit_spike::op_tri_count(root), vol.to_bits());
    for segments in [64, 128, 256] {
        // warm once (first call pays lazy init), then time 3 runs
        web_fit_spike::bench_boolean(segments);
        let start = std::time::Instant::now();
        const RUNS: u32 = 3;
        let mut tris = 0;
        for _ in 0..RUNS {
            tris = web_fit_spike::bench_boolean(segments);
        }
        let per = start.elapsed() / RUNS;
        println!("segments {segments}: {tris} tris, {per:?}/op");
    }
}
