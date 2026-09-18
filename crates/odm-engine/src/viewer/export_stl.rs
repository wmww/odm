//! File ▸ Export STL…: the active tab's solids, as an STL file.
//!
//! The dialog exports a **snapshot**: the tab's published result at the
//! moment it opened, pinned in the store. The tab may keep playing or
//! rebuilding behind it; header, size line and file always agree, and it is
//! exactly what was on screen. So there is no build here, no JS and no build
//! gate — the worker thread only needs the kernel and the store.

use super::browse::Browser;
use super::new::units_ui;
use crate::stl::{StlOptions, StlReport, export_stl, fmt_size, size_warning};
use crate::theme;
use eframe::egui;
use odm_build::{Units, View};
use odm_ir::Node;
use odm_kernel::{CancelToken, Kernel};
use odm_store::{RootPin, Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Fixed dialog size — see `theme::dialog` on why the width is fixed.
const WIDTH: f32 = 420.0;

pub enum Outcome {
    /// Still open.
    Idle,
    /// Cancelled, finished, or dismissed mid-export (which cancels it).
    Closed,
}

/// What the dialog remembers between uses, for the session: a new session
/// starts from the project's unit, union on, browsing the project.
#[derive(Clone, Default)]
pub struct Prefs {
    /// Where the last STL went.
    pub dir: Option<PathBuf>,
    /// `None` = the project's.
    pub units: Option<Units>,
    /// `None` = on.
    pub union: Option<bool>,
}

/// What is being exported: a tab's published result, as of the dialog opening.
pub struct Snapshot {
    pub view: View,
    pub root: Node,
    /// Keeps `root`'s subtree in the store; shared with the worker.
    pub pin: Arc<RootPin>,
    /// World bounds of the scene, model units — the live size line's source.
    pub bounds: Option<([f64; 3], [f64; 3])>,
    /// A newer build was pending when the snapshot was taken.
    pub stale: bool,
}

enum Stage {
    Pick,
    /// The worker is exporting; the receiver gets its one result.
    Running(Receiver<Result<StlReport, String>>, CancelToken),
    Done(StlReport),
}

pub struct StlDialog {
    /// `<project> <view path>`, for the file's header.
    label: String,
    project_units: Units,
    snapshot: Snapshot,
    store: Arc<Store>,
    kernel: Arc<Kernel>,
    browser: Browser,
    /// The "File:" field.
    name: String,
    opts: StlOptions,
    /// The existing file the user has been asked about replacing.
    asked: Option<PathBuf>,
    stage: Stage,
}

impl StlDialog {
    pub fn new(
        project: &Path,
        label: String,
        project_units: Units,
        snapshot: Snapshot,
        store: Arc<Store>,
        kernel: Arc<Kernel>,
        prefs: &Prefs,
    ) -> StlDialog {
        let dir = prefs.dir.as_deref().filter(|d| d.is_dir()).unwrap_or(project);
        StlDialog {
            label,
            project_units,
            name: default_name(&snapshot.view.path),
            snapshot,
            store,
            kernel,
            browser: Browser::at(dir),
            opts: StlOptions {
                units: prefs.units.unwrap_or(project_units),
                union: prefs.union.unwrap_or(true),
            },
            asked: None,
            stage: Stage::Pick,
        }
    }

    /// The choices to carry to the next export this session.
    pub fn remember(&self, prefs: &mut Prefs) {
        prefs.units = (self.opts.units != self.project_units).then_some(self.opts.units);
        prefs.union = Some(self.opts.union);
        if let Stage::Done(report) = &self.stage {
            prefs.dir = report.path.parent().map(Path::to_path_buf);
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        self.poll();
        let res = theme::dialog(ctx, "export-stl", "Export STL", WIDTH, |ui| self.body(ui));
        let outcome = if res.dismissed { Outcome::Closed } else { res.inner };
        if let (Outcome::Closed, Stage::Running(_, cancel)) = (&outcome, &self.stage) {
            cancel.cancel();
        }
        outcome
    }

    /// Collect the worker's result, if it is in.
    fn poll(&mut self) {
        if let Stage::Running(rx, _) = &self.stage {
            let result = match rx.try_recv() {
                Err(TryRecvError::Empty) => return,
                Ok(result) => result,
                Err(TryRecvError::Disconnected) => Err("the export thread died".to_owned()),
            };
            match result {
                Ok(report) => self.stage = Stage::Done(report),
                Err(e) => {
                    self.browser.report(e);
                    self.stage = Stage::Pick;
                }
            }
        }
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Outcome {
        match &self.stage {
            Stage::Pick => self.pick_ui(ui),
            Stage::Running(..) => {
                ui.add_space(8.0);
                ui.label("Exporting…");
                ui.add_space(8.0);
                Outcome::Idle
            }
            Stage::Done(report) => done_ui(ui, report),
        }
    }

    fn pick_ui(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Idle;
        ui.label(header_line(&self.snapshot.view));
        if self.snapshot.stale {
            let note = "a newer build is pending — exporting what is shown";
            ui.label(egui::RichText::new(note).color(theme::WEAK_TEXT));
        }
        ui.add_space(4.0);
        self.browser.header_ui(ui);
        ui.add_space(4.0);
        let hit = self.browser.list_ui(ui);
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("File:");
            let width = ui.available_width() - 4.0;
            theme::text_edit(ui, "export-stl-name", &mut self.name, width, "part.stl");
        });

        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Units:");
            let project = Some(self.project_units);
            if let Some(units) = units_ui(ui, "export-stl-units", self.opts.units, project) {
                self.opts.units = units;
            }
        });
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 18.0),
            egui::Sense::hover(),
        );
        let text = "Union overlapping solids";
        if theme::check_box(ui, "export-stl-union", rect, self.opts.union, text).clicked() {
            self.opts.union = !self.opts.union;
        }
        if let Some((text, warn)) = size_line(self.snapshot.bounds, self.opts.units) {
            let color = if warn { theme::WARN } else { theme::WEAK_TEXT };
            ui.label(egui::RichText::new(text).color(color));
        }
        self.browser.error_ui(ui);

        ui.add_space(5.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() - 130.0);
            if theme::button(ui, "Export").clicked() || confirm {
                match self.resolve() {
                    Ok(out) if out.exists() && self.asked.as_ref() != Some(&out) => {
                        let name = out.file_name().unwrap_or_default().to_string_lossy();
                        self.browser.report(format!("{name} exists — Export again to replace it"));
                        self.asked = Some(out);
                    }
                    Ok(out) => self.start(out, ui.ctx()),
                    Err(e) => self.browser.report(e),
                }
            }
            if theme::button(ui, "Cancel").clicked() {
                outcome = Outcome::Closed;
            }
        });

        // Left until the dialog is fully drawn: navigating mid-layout would
        // relist under the rows still being iterated.
        if let Some(dir) = hit.entered {
            self.browser.navigate(dir);
        }
        outcome
    }

    /// The file to write, if the fields make sense. STL files are inert to
    /// the scanner, so anywhere is fine — the project included.
    fn resolve(&self) -> Result<PathBuf, String> {
        let dir = self.browser.dir();
        if !dir.is_dir() {
            return Err(format!("{} is not a directory", dir.display()));
        }
        let name = self.name.trim();
        if name.is_empty() {
            return Err("the file needs a name".to_owned());
        }
        if Path::new(name).file_name() != Some(name.as_ref()) {
            return Err("the name must be one file name, with no '/' in it".to_owned());
        }
        let has_ext = Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("stl"));
        let out = dir.join(if has_ext { name.to_owned() } else { format!("{name}.stl") });
        if out.is_dir() {
            return Err(format!("{} is a folder", out.display()));
        }
        Ok(out)
    }

    fn start(&mut self, out: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = CancelToken::new();
        let (store, kernel) = (self.store.clone(), self.kernel.clone());
        let (root, pin) = (self.snapshot.root.clone(), self.snapshot.pin.clone());
        let (label, opts, token, ctx) = (self.label.clone(), self.opts, cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = export_stl(&store, &kernel, &root, &label, &out, &opts, Some(&token));
            drop(pin);
            let _ = tx.send(result);
            // The dialog only polls when a frame happens to run; make one.
            ctx.request_repaint();
        });
        self.stage = Stage::Running(rx, cancel);
    }
}

fn done_ui(ui: &mut egui::Ui, report: &StlReport) -> Outcome {
    let mut outcome = Outcome::Idle;
    ui.add_space(4.0);
    ui.label(report.line());
    for warning in &report.warnings {
        ui.label(egui::RichText::new(format!("warning: {warning}")).color(theme::WARN));
    }
    ui.add_space(4.0);
    let path = report.path.display().to_string();
    let link = egui::Label::new(egui::RichText::new(&path).color(theme::WEAK_TEXT))
        .sense(egui::Sense::click());
    if ui.add(link).on_hover_text("click to copy").clicked() {
        ui.ctx().copy_text(path);
    }
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.add_space(ui.available_width() - 65.0);
        if theme::button(ui, "Close").clicked() {
            outcome = Outcome::Closed;
        }
    });
    outcome
}

/// `arm.stl` for `parts/arm.js`.
fn default_name(view_path: &str) -> String {
    let stem = Path::new(view_path).file_stem().unwrap_or_default().to_string_lossy();
    format!("{stem}.stl")
}

/// `Exporting parts/arm.js  t=0.75` — which view, at which instant.
fn header_line(view: &View) -> String {
    let mut inputs = view.args.clone();
    inputs.extend(view.cascade.clone());
    let mut overrides = serde_json::Map::new();
    overrides.insert("inputs".into(), serde_json::Value::Object(inputs));
    let caption = crate::commands::caption_for(&overrides);
    match caption.is_empty() {
        true => format!("Exporting {}", view.path),
        false => format!("Exporting {}  {caption}", view.path),
    }
}

/// `0.08 × 0.06 × 0.0042 m = 80 × 60 × 4.2 mm`, and whether the size looks
/// like the wrong unit. No kernel work: the scene's bounds times the unit.
fn size_line(bounds: Option<([f64; 3], [f64; 3])>, units: Units) -> Option<(String, bool)> {
    let (min, max) = bounds?;
    let size = [0, 1, 2].map(|k| max[k] - min[k]);
    let mm = size.map(|s| s * units.to_mm());
    let text = match units {
        Units::Mm => format!("{} mm", fmt_size(mm)),
        _ => format!("{} {units} = {} mm", fmt_size(size), fmt_size(mm)),
    };
    Some((text, size_warning(mm).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use odm_ir::Transform;
    use odm_store::Object;
    use serde_json::json;

    fn view(path: &str) -> View {
        View { path: path.into(), args: Default::default(), cascade: Default::default() }
    }

    #[test]
    fn the_header_says_which_view_at_which_instant() {
        let mut v = view("parts/arm.js");
        assert_eq!(header_line(&v), "Exporting parts/arm.js");
        v.cascade.insert("t".into(), json!(0.75));
        assert_eq!(header_line(&v), "Exporting parts/arm.js  t=0.75");
        assert_eq!(default_name("parts/arm.js"), "arm.stl");
    }

    #[test]
    fn the_size_line_converts_and_flags_a_wrong_unit() {
        let bounds = Some(([0.0; 3], [0.08, 0.06, 0.0042]));
        let (text, warn) = size_line(bounds, Units::M).unwrap();
        assert_eq!((text.as_str(), warn), ("0.08 × 0.06 × 0.0042 m = 80 × 60 × 4.2 mm", false));
        let (text, warn) = size_line(bounds, Units::Mm).unwrap();
        assert_eq!((text.as_str(), warn), ("0.08 × 0.06 × 0.0042 mm", true));
        assert!(size_line(None, Units::Mm).is_none());
    }

    /// A 20-unit cube's snapshot, in a dialog browsing `dir`.
    fn dialog(dir: &Path, prefs: &Prefs, project_units: Units) -> StlDialog {
        let store = Store::new();
        let kernel = Kernel::new(store.clone());
        let mesh = kernel.cube(20.0, 20.0, 20.0, false).unwrap();
        let root = Node { mesh: Some(mesh), transform: Transform::IDENTITY, ..Node::default() };
        let hash = store.put(Object::Node(root.clone()));
        let snapshot = Snapshot {
            view: view("parts/arm.js"),
            root,
            pin: Arc::new(store.pin_root(hash)),
            bounds: Some(([0.0; 3], [20.0; 3])),
            stale: true,
        };
        StlDialog::new(dir, "proj parts/arm.js".into(), project_units, snapshot, store, kernel, prefs)
    }

    fn frame(ctx: &egui::Context, dialog: &mut StlDialog, events: Vec<egui::Event>) -> Outcome {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0))),
            events,
            ..Default::default()
        };
        ctx.begin_pass(input);
        let outcome = dialog.ui(ctx);
        let _ = ctx.end_pass();
        outcome
    }

    fn enter() -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    /// Enter exports to `<stem>.stl` in the browsed folder, asks once before
    /// replacing it, and the choices carry to the next dialog.
    #[test]
    fn enter_exports_and_a_second_export_asks_before_replacing() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut prefs = Prefs::default();
        let mut d = dialog(dir.path(), &prefs, Units::Mm);
        assert!(d.opts == StlOptions { units: Units::Mm, union: true });
        d.opts.units = Units::In;

        frame(&ctx, &mut d, enter());
        let out = dir.path().join("arm.stl");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !matches!(d.stage, Stage::Done(_)) {
            assert!(std::time::Instant::now() < deadline, "the export never finished");
            std::thread::sleep(std::time::Duration::from_millis(5));
            frame(&ctx, &mut d, Vec::new());
        }
        let Stage::Done(report) = &d.stage else { unreachable!() };
        assert_eq!(report.line(), "508 × 508 × 508 mm · 1 body · 12 triangles");
        assert_eq!(report.path, out);
        assert_eq!(std::fs::read(&out).unwrap().len(), 84 + 12 * 50);

        d.remember(&mut prefs);
        assert_eq!((prefs.units, prefs.union), (Some(Units::In), Some(true)));
        assert_eq!(prefs.dir.as_deref(), Some(dir.path()));

        // Next time: same folder, same unit; the file is there now.
        let mut d = dialog(Path::new("/"), &prefs, Units::Mm);
        assert_eq!((d.browser.dir(), d.opts.units), (dir.path(), Units::In));
        frame(&ctx, &mut d, enter());
        assert!(matches!(d.stage, Stage::Pick), "the first Enter only asks");
        assert_eq!(d.asked.as_ref(), Some(&out));
        frame(&ctx, &mut d, enter());
        assert!(matches!(d.stage, Stage::Running(..)));
    }

    /// Dismissing mid-export cancels the worker's token.
    #[test]
    fn dismissing_a_running_export_cancels_it() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut d = dialog(dir.path(), &Prefs::default(), Units::Mm);
        let (_tx, rx) = std::sync::mpsc::channel();
        let cancel = CancelToken::new();
        d.stage = Stage::Running(rx, cancel.clone());
        frame(&ctx, &mut d, Vec::new());
        let escape = vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        assert!(matches!(frame(&ctx, &mut d, escape), Outcome::Closed));
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn the_file_field_takes_a_bare_name_and_refuses_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = dialog(dir.path(), &Prefs::default(), Units::Mm);
        d.name = "bracket".into();
        assert_eq!(d.resolve().unwrap(), dir.path().join("bracket.stl"));
        d.name = "bracket.STL".into();
        assert_eq!(d.resolve().unwrap(), dir.path().join("bracket.STL"));
        d.name = "../bracket.stl".into();
        assert!(d.resolve().unwrap_err().contains("one file name"));
        d.name = " ".into();
        assert!(d.resolve().unwrap_err().contains("needs a name"));
    }
}
