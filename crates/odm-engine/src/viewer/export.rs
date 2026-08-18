//! File ▸ Export Web…: turn the project into a static site, right where the
//! user says. Exports the active tab's view — path, set inputs and all — so
//! the page opens on what the viewer was showing.
//!
//! The export itself runs on a background thread: meta extraction can hang on
//! a broken doohickey for its full 10s timeout, and the UI must not. It reuses
//! the process's one `JsEnv` — creating a second V8 snapshot while the build
//! thread executes JS aborts the process (`JsEnv::new` says so).

use super::browse::Browser;
use crate::session::is_project;
use crate::theme;
use eframe::egui;
use odm_build::View;
use odm_export::ExportReport;
use odm_js::JsEnv;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Fixed dialog size — see `theme::dialog` on why the width is fixed.
const WIDTH: f32 = 420.0;

pub enum Outcome {
    /// Still open.
    Idle,
    /// Cancelled, exported, or dismissed mid-export (the export finishes in
    /// the background either way; only the report goes unshown).
    Closed,
}

enum Stage {
    /// Browsing for where the site should go.
    Pick,
    /// The worker thread is exporting; the receiver gets its one result.
    Running(Receiver<Result<ExportReport, String>>),
    /// Done: showing the report until the user closes it.
    Done(ExportReport),
}

pub struct ExportDialog {
    project: PathBuf,
    /// The active tab's view at the moment the dialog opened.
    view: View,
    env: Arc<JsEnv>,
    browser: Browser,
    /// The "Folder:" field: the site folder to make in the browsed directory.
    name: String,
    stage: Stage,
}

impl ExportDialog {
    pub fn new(project: &Path, view: View, env: Arc<JsEnv>) -> ExportDialog {
        // Offer a sibling of the project, named after it.
        let name = project
            .file_name()
            .map(|n| format!("{}-web", n.to_string_lossy()))
            .unwrap_or_else(|| "site".to_owned());
        ExportDialog {
            project: project.to_path_buf(),
            view,
            env,
            browser: Browser::beside(project),
            name,
            stage: Stage::Pick,
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        self.poll();
        let res = theme::dialog(ctx, "export-web", "Export Web", WIDTH, |ui| self.body(ui));
        if res.dismissed { Outcome::Closed } else { res.inner }
    }

    /// Collect the worker's result, if it is in.
    fn poll(&mut self) {
        if let Stage::Running(rx) = &self.stage {
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
            Stage::Running(_) => {
                ui.add_space(8.0);
                ui.label("Exporting…");
                ui.add_space(8.0);
                Outcome::Idle
            }
            Stage::Done(report) => {
                let mut outcome = Outcome::Idle;
                ui.add_space(4.0);
                let s = if report.files == 1 { "" } else { "s" };
                ui.label(format!(
                    "Exported {} doohickey{s} to {}.",
                    report.files,
                    report.out.display()
                ));
                for warning in &report.warnings {
                    ui.label(egui::RichText::new(format!("warning: {warning}")).color(theme::ERROR));
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new(
                    "Serve the folder with any static file server and open it \
                     in a WebGPU-capable browser.",
                ).color(theme::WEAK_TEXT));
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() - 65.0);
                    if theme::button(ui, "Close").clicked() {
                        outcome = Outcome::Closed;
                    }
                });
                outcome
            }
        }
    }

    fn pick_ui(&mut self, ui: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Idle;
        self.browser.header_ui(ui);
        ui.add_space(4.0);
        let hit = self.browser.list_ui(ui);
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Folder:");
            let width = ui.available_width() - 4.0;
            theme::text_edit(ui, "export-name", &mut self.name, width, "empty: into this folder");
        });
        self.browser.error_ui(ui);

        ui.add_space(5.0);
        let confirm = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() - 130.0);
            if theme::button(ui, "Export").clicked() || confirm {
                match self.resolve() {
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
            match is_project(&dir) {
                true => self.browser.report(inside().to_owned()),
                false => self.browser.navigate(dir),
            }
        }
        outcome
    }

    /// Where the site would go, if it can go there. Re-exporting over an
    /// existing site is the normal round trip, so an existing folder is fine.
    fn resolve(&self) -> Result<PathBuf, String> {
        let dir = self.browser.dir();
        if !dir.is_dir() {
            return Err(format!("{} is not a directory", dir.display()));
        }
        let name = self.name.trim();
        let out = if name.is_empty() {
            dir.to_path_buf()
        } else {
            if Path::new(name).file_name() != Some(name.as_ref()) {
                return Err("the name must be one folder name, with no '/' in it".to_owned());
            }
            dir.join(name)
        };
        // The export writes .js files, and every .js file under a project
        // belongs to the project: a site in one would be built as doohickeys.
        if out.ancestors().any(is_project) {
            return Err(inside().to_owned());
        }
        Ok(out)
    }

    fn start(&mut self, out: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let project = self.project.clone();
        let view = self.view.clone();
        let env = self.env.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let opts = odm_export::ExportOptions { view: Some(view), ..Default::default() };
            let _ = tx.send(odm_export::export_web_with_env(&project, &out, &opts, &env));
            // The dialog only polls when a frame happens to run; make one.
            ctx.request_repaint();
        });
        self.stage = Stage::Running(rx);
    }
}

fn inside() -> &'static str {
    "a site cannot go inside a project"
}
