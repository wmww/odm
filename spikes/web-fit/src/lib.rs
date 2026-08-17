//! Web-export fit spike (plans/web-export.md phase 0, throwaway).
//!
//! Proves three joints in one module:
//! - odm-kernel (Manifold C++ via wasm-cxx-shim) + odm-store + odm-ir all
//!   linked into ONE wasm32-unknown-unknown module — no emscripten, no glue
//!   between two wasm modules.
//! - the ops seam shape: plain sync exports backed by Kernel/Store, solids
//!   as opaque handles, mesh data crossing as a typed-array view.
//! - the reentrant build round trip: JS host calls `scheduler_build` (the
//!   Rust orchestration), which calls the imported JS executor
//!   (`js_run_build`, standing in for running a doohickey), which calls ops
//!   back into this module. That is the native `run_build` control flow with
//!   V8 replaced by the page's own JS.

use odm_ir::{Hash, Transform};
use odm_kernel::{BoolOp, Kernel};
use odm_store::{Object, Store};
use std::cell::RefCell;
use std::sync::Arc;

struct Ctx {
    kernel: Arc<Kernel>,
    store: Arc<Store>,
    /// Solid handle table: JS sees u32 indices, hashes stay Rust-side.
    handles: Vec<Hash>,
    /// Staging buffer for mesh positions read by JS as a memory view.
    mesh_buf: Vec<f64>,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

fn with_ctx<R>(f: impl FnOnce(&mut Ctx) -> R) -> R {
    CTX.with(|c| f(c.borrow_mut().as_mut().expect("odm_init not called")))
}

/// Content-addressed like the store: the same hash gets the same handle,
/// so identical rebuilds see identical handles.
fn handle(c: &mut Ctx, h: Hash) -> u32 {
    if let Some(i) = c.handles.iter().position(|&x| x == h) {
        return i as u32;
    }
    c.handles.push(h);
    (c.handles.len() - 1) as u32
}

fn translation(dx: f64, dy: f64, dz: f64) -> Transform {
    let mut t = Transform::IDENTITY;
    t.0[12] = dx;
    t.0[13] = dy;
    t.0[14] = dz;
    t
}

#[unsafe(no_mangle)]
pub extern "C" fn odm_init() {
    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    CTX.with(|c| {
        *c.borrow_mut() = Some(Ctx { kernel, store, handles: Vec::new(), mesh_buf: Vec::new() })
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn op_solid_box(x: f64, y: f64, z: f64) -> u32 {
    with_ctx(|c| {
        let h = c.kernel.cube(x, y, z, true).expect("cube");
        handle(c, h)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn op_solid_sphere(radius: f64, segments: i32) -> u32 {
    with_ctx(|c| {
        let h = c.kernel.sphere(radius, segments).expect("sphere");
        handle(c, h)
    })
}

/// op: 0 = union, 1 = difference, 2 = intersection. Operand `b` is
/// translated by (dx, dy, dz) — exercises per-operand transforms.
#[unsafe(no_mangle)]
pub extern "C" fn op_boolean(op: u32, a: u32, b: u32, dx: f64, dy: f64, dz: f64) -> u32 {
    with_ctx(|c| {
        let op = match op {
            0 => BoolOp::Union,
            1 => BoolOp::Difference,
            _ => BoolOp::Intersection,
        };
        let (ha, hb) = (c.handles[a as usize], c.handles[b as usize]);
        let h = c
            .kernel
            .boolean(op, &[(ha, Transform::IDENTITY), (hb, translation(dx, dy, dz))], None)
            .expect("boolean");
        handle(c, h)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn op_volume(h: u32) -> f64 {
    with_ctx(|c| c.kernel.volume(c.handles[h as usize]).expect("volume"))
}

#[unsafe(no_mangle)]
pub extern "C" fn op_tri_count(h: u32) -> u32 {
    with_ctx(|c| match c.store.get(c.handles[h as usize]).as_deref() {
        Some(Object::Mesh(m)) => (m.indices.len() / 3) as u32,
        _ => u32::MAX,
    })
}

/// Stage a solid's mesh positions for reading; returns f64 count.
#[unsafe(no_mangle)]
pub extern "C" fn op_mesh_load(h: u32) -> u32 {
    with_ctx(|c| {
        let Some(obj) = c.store.get(c.handles[h as usize]) else { return 0 };
        match obj.as_ref() {
            Object::Mesh(m) => {
                c.mesh_buf = m.positions.clone();
                c.mesh_buf.len() as u32
            }
            _ => 0,
        }
    })
}

/// Pointer into wasm memory for the staged positions (Float64Array view).
#[unsafe(no_mangle)]
pub extern "C" fn op_mesh_ptr() -> *const f64 {
    with_ctx(|c| c.mesh_buf.as_ptr())
}

/// Perf probe: difference of two offset high-segment spheres. Returns the
/// result's tri count. Same path timed native (src/bin/native_bench.rs) and
/// in wasm.
#[unsafe(no_mangle)]
pub extern "C" fn bench_boolean(segments: i32) -> u32 {
    with_ctx(|c| {
        let a = c.kernel.sphere(1.0, segments).expect("sphere a");
        let b = c.kernel.sphere(1.0, segments).expect("sphere b");
        let h = c
            .kernel
            .boolean(
                BoolOp::Difference,
                &[(a, Transform::IDENTITY), (b, translation(0.5, 0.3, 0.2))],
                None,
            )
            .expect("boolean");
        let n = match c.store.get(h).as_deref() {
            Some(Object::Mesh(m)) => (m.indices.len() / 3) as u32,
            _ => 0,
        };
        c.kernel.clear_cache();
        n
    })
}

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    /// The JS executor: runs "doohickey" code host-side, returns the root
    /// solid handle. Stands in for odm-js's run_build.
    fn js_run_build(build_id: u32) -> u32;
}

/// The Rust build orchestration entry: JS calls this, this calls back into
/// JS, which calls ops back into this module. Wasm→JS→wasm reentrancy is
/// the load-bearing bit.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn scheduler_build(build_id: u32) -> u32 {
    unsafe { js_run_build(build_id) }
}
