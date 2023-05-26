use crate::plugins::logging::LogEntry;
use bevy::log;
use bevy_egui::egui;
use std::collections::VecDeque;

pub struct LogsViewer<'a> {
    entries: &'a VecDeque<LogEntry>,
}

impl<'a> LogsViewer<'a> {
    pub fn new(entries: &'a VecDeque<LogEntry>) -> Self {
        Self { entries }
    }

    pub fn show(self, ui: &mut egui::Ui) {
        let log_entries = self.entries;

        let text_style = egui::TextStyle::Body;
        let row_height = ui.text_style_height(&text_style);
        let total_rows = self.entries.len();
        egui::ScrollArea::both()
            .stick_to_bottom(true)
            .auto_shrink([false; 2])
            .show_rows(ui, row_height, total_rows, |ui, row_range| {
                for row in row_range {
                    let entry = &log_entries[row];
                    // Get rid of spacing between rows (caused by `horizontal`) to avoid issues with
                    // `show_rows` (due to mismatching `row_height`).
                    ui.style_mut().spacing.interact_size.y = 0.0;
                    ui.horizontal(|ui| {
                        ui.label(entry.timestamp.trim());
                        let color = match entry.level {
                            log::Level::TRACE => egui::Color32::GRAY,
                            log::Level::DEBUG => egui::Color32::BLUE,
                            log::Level::INFO => egui::Color32::WHITE,
                            log::Level::WARN => egui::Color32::GOLD,
                            log::Level::ERROR => egui::Color32::RED,
                        };
                        ui.colored_label(color, entry.level.as_str());
                        ui.label(entry.message.trim());
                    });
                }
            });
    }
}
