//! The engine as the viewer core sees it: submit a view for a slot, read the
//! slot's last published result, pick into a scene. Same addressing as CLI
//! queries — everything is a view. The desktop engine implements this over
//! `EngineState`; the web host will implement it over the wasm build loop.

use odm_build::{InputReport, ReportEntry, View};
use odm_render::Instance;
use odm_store::{LogLine, Object, Store};
use std::sync::Arc;

/// Last published build of one active view slot. Last-good semantics — a
/// failed build updates `error` but keeps the previous root.
#[derive(Clone, Default)]
pub struct Published {
    /// Bumped whenever anything here changes; the viewer polls it.
    pub revision: u64,
    pub generation: u64,
    /// The view this result was built for.
    pub view: View,
    /// Root hash plus the object itself: holding the `Arc` keeps the root
    /// alive across store GCs, so the viewer never reads an unrooted hash.
    pub root: Option<(odm_ir::Hash, Arc<Object>)>,
    pub error: Option<String>,
    /// Console output of the last build attempt — success or failure, memo
    /// hits replay theirs — as (part path, line). Latest-attempt
    /// semantics, unlike `root`'s last-good.
    pub logs: Arc<Vec<(String, LogLine)>>,
    pub building: bool,
    /// Fall-through report of the last successful build: the view-settable
    /// inputs (the input panel's data source).
    pub report: Arc<InputReport>,
    /// The target's declared inputs at the last *failed* build, when its
    /// meta was still extractable — what [`crate::Tab::prune_stale_args`]
    /// drops stale pinned args against. `None` on success (the report is
    /// fresher) or when the failure precedes meta (scan error, broken meta).
    pub declared: Option<Arc<Vec<ReportEntry>>>,
}

/// What the core asks of its host. Everything a tab draws comes back through
/// [`Engine::published`]; everything the user changes goes out through
/// [`Engine::set_view`]. The host owns view lifecycle beyond that (creating
/// and removing slots is chrome, not read-side).
pub trait Engine {
    /// Submit (or replace) the view a slot keeps built; latest wins.
    fn set_view(&self, slot: &str, view: View);

    /// The slot's last published result.
    fn published(&self, slot: &str) -> Published;

    /// The store published roots (and their subtrees and meshes) live in.
    fn store(&self) -> &Store;

    /// Nearest solid surface along a world-space ray: (node id, name).
    fn raycast(
        &self,
        instances: &[Instance],
        origin: [f64; 3],
        dir: [f64; 3],
    ) -> Option<(String, Option<String>)>;

    /// The selection of the tab the user is looking at, for hosts that
    /// report it onward (the desktop CLI's `status`).
    fn set_selection(&self, sel: &[(String, Option<String>)]) {
        let _ = sel;
    }
}
