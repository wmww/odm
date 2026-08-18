//! The console pane: the last build attempt's output, devtools style. A host
//! gives it a place to sit — the desktop viewer makes it one tab of its
//! bottom dock — and labels that place with [`console_tab`].

use crate::tab::Tab;
use crate::theme;
use eframe::egui;
use odm_store::LogLevel;

/// The console's label and color for whatever the host tabs it under: how
/// much the last build attempt had to say, red for a thrown error and amber
/// for a warning logged, so a failed build says so from wherever it is.
pub fn console_tab(tab: &Tab) -> (String, egui::Color32) {
    let logs = &tab.published.logs;
    let error = tab.published.error.is_some();
    let color = if error {
        theme::ERROR
    } else if logs.iter().any(|(_, l)| matches!(l.level, LogLevel::Warn | LogLevel::Error)) {
        theme::WARN
    } else {
        theme::TEXT
    };
    let label = match logs.len() + error as usize {
        0 => "Output".to_owned(),
        n => format!("Output ({n})"),
    };
    (label, color)
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
