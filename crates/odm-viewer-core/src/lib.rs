//! The viewer's read side, reusable across hosts: camera + viewport painting,
//! scene tree with click-select, generated input panel, `t` transport,
//! console pane, and the theme — everything needed to *look at* one built
//! view. Parameterized over [`Engine`], the small interface a host implements
//! (submit a view, read its published result, pick). The desktop viewer in
//! odm-engine wraps this in its chrome (sessions, menu bar, tabs, chat,
//! dialogs); the web export will be a second, smaller host.
//!
//! Dependency rule: nothing here may touch odm-js, `EngineState`, sockets or
//! the file watcher — this crate is meant to compile to wasm once odm-build's
//! JS-executor seam makes odm-js optional (see plans/web-export.md phase 1).

pub mod icons;
pub mod inputs;
pub mod theme;

mod camera;
mod console;
mod engine;
mod tab;
mod tree;
mod viewer;
mod viewport;

pub use camera::{FOV_Y_DEG, Orbit};
pub use console::{console_tab, console_ui};
pub use engine::{Engine, Published};
pub use tab::{SceneCache, Section, Tab};
pub use tree::selection_covers;
pub use viewer::{PANEL_SHARE, Viewer, blank_viewport};
pub use viewport::OffscreenTarget;
