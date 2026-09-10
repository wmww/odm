//! Shared test-renderer construction.
//!
//! A render test that quietly returns when there is no GPU is a test that
//! passes without running, so the default is to fail loudly; skipping is an
//! explicit opt-in for a machine that genuinely has no adapter.

use odm_render::Renderer;
use std::sync::OnceLock;

/// The test renderer. Panics without a GPU adapter unless `ODM_TEST_NO_GPU=1`
/// (then returns `None` so the caller skips).
pub fn renderer() -> Option<Renderer> {
    match Renderer::new() {
        Ok(r) => {
            static ANNOUNCED: OnceLock<()> = OnceLock::new();
            ANNOUNCED.get_or_init(|| eprintln!("render tests on adapter: {}", r.adapter_description()));
            Some(r)
        }
        Err(e) if std::env::var("ODM_TEST_NO_GPU").as_deref() == Ok("1") => {
            eprintln!("ODM_TEST_NO_GPU=1: skipping a render test ({e})");
            None
        }
        Err(e) => panic!(
            "no GPU adapter: {e}. Set ODM_TEST_NO_GPU=1 to skip the render tests \
             on a machine that genuinely has none."
        ),
    }
}
