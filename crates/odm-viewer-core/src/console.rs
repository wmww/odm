//! The console pane: the last build attempt's output, devtools style. A host
//! gives it a place to sit — the desktop viewer makes it one tab of its
//! bottom dock — and labels that place with [`console_tab`].

use crate::tab::Tab;
use crate::theme;
use eframe::egui;
use odm_store::LogLevel;

/// The console's tab label, for whatever the host tabs it under. A lamp says
/// what the last build attempt had to say: none at all when it said nothing,
/// gray for plain logs, amber for a warning or a logged error, red for a
/// thrown one — so a failed build says so from wherever the console sits.
pub fn console_tab(tab: &Tab) -> theme::StripTab {
    let logs = &tab.published.logs;
    let lamp = if tab.published.error.is_some() {
        Some(theme::ERROR)
    } else if logs.iter().any(|(_, l)| matches!(l.level, LogLevel::Warn | LogLevel::Error)) {
        Some(theme::WARN)
    } else if logs.is_empty() {
        None
    } else {
        Some(theme::LAMP_QUIET)
    };
    let tab = theme::StripTab::new("Output", theme::TEXT);
    match lamp {
        Some(color) => tab.lamp(color),
        None => tab,
    }
}

/// One console, browser-devtools style: the last build attempt's output in
/// order (latest-attempt semantics), and — when the build failed — the
/// thrown error as the final entry, which is where it fell
/// chronologically. Presentation-only merge: `error` stays its own field
/// everywhere else (tab badge, agent surfaces, last-good scene semantics).
pub fn console_ui(ui: &mut egui::Ui, tab: &Tab) {
    let logs = &tab.published.logs;
    let error = &tab.published.error;
    let size = egui::vec2(ui.available_width(), ui.available_height().max(24.0));
    theme::list_box(ui, "console", size, egui::Vec2b::new(false, true), |ui| {
        if logs.is_empty() && error.is_none() {
            ui.label(egui::RichText::new("The build had nothing to say.").color(theme::WEAK_TEXT));
        }
        for (path, line) in logs.iter() {
            let color = match line.level {
                LogLevel::Error => theme::ERROR,
                LogLevel::Warn => theme::WARN,
                LogLevel::Debug => theme::WEAK_TEXT,
                LogLevel::Log => theme::TEXT,
            };
            ui.label(
                egui::RichText::new(format!("{path}: {}", line.message)).monospace().color(color),
            );
        }
        if let Some(err) = error {
            ui.label(egui::RichText::new(err).monospace().color(theme::ERROR));
        }
    });
}
