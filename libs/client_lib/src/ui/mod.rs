use bevy::{
    ecs::{
        query::With,
        system::{Query, ResMut},
    },
    window::{PrimaryWindow, Window},
};
use bevy_egui::{
    egui::{self, Ui},
    EguiSettings,
};
use mr_shared_lib::game::components::{PlayerDirection, Position};

pub mod builder_ui;
pub mod debug_ui;
pub mod main_menu_ui;
pub mod overlay_ui;
pub mod player_ui;
pub mod side_panel;

mod widgets;

pub fn set_ui_scale_factor_system(
    mut egui_settings: ResMut<EguiSettings>,
    primary_window_query: Query<&Window, With<PrimaryWindow>>,
) {
    if let Ok(window) = primary_window_query.get_single() {
        if window.scale_factor() % 2.0 > 0.0 {
            egui_settings.scale_factor = 1.0 / window.scale_factor();
        }
    }
}

pub trait MuddleInspectable {
    fn inspect(&self, ui: &mut Ui);

    fn inspect_mut(&mut self, ui: &mut Ui) {
        self.inspect(ui);
    }
}

impl MuddleInspectable for PlayerDirection {
    fn inspect(&self, ui: &mut Ui) {
        let latest_value = self
            .buffer
            .get_with_extrapolation(self.buffer.end_frame())
            .map(|(_, v)| *v)
            .map(|v| format!("[{: <5.2};{: >5.2}]", v.x, v.y))
            .unwrap_or_else(|| "[None]".to_owned());

        egui::CollapsingHeader::new(format!(
            "Direction {}  -  ([{}; {}] ({}/{})",
            latest_value,
            self.buffer.start_frame().value(),
            self.buffer.end_frame().value(),
            self.buffer.limit(),
            self.buffer.len(),
        ))
        .id_source("player direction buffer")
        .default_open(false)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    for (frame_number, value) in self.buffer.iter().rev() {
                        let value = value
                            .map(|v| format!("[{: <5.2};{: >5.2}]", v.x, v.y))
                            .unwrap_or_else(|| "[None]".to_owned());
                        ui.label(format!("{}: {}", frame_number.value(), value));
                    }
                });
        });
    }
}

impl MuddleInspectable for Position {
    fn inspect(&self, ui: &mut Ui) {
        let first_value = self
            .buffer
            .last()
            .map(|v| format!("[{: <5.2};{: >5.2}]", v.x, v.y))
            .unwrap_or_else(|| "[None]".to_owned());

        egui::CollapsingHeader::new(format!(
            "Position {}  -  ([{}; {}] ({}/{})",
            first_value,
            self.buffer.start_frame().value(),
            self.buffer.end_frame().value(),
            self.buffer.limit(),
            self.buffer.len(),
        ))
        .id_source("position buffer")
        .default_open(false)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    for (frame_number, value) in self.buffer.iter().rev() {
                        ui.label(format!(
                            "{}: [{: <5.2};{: >5.2}]",
                            frame_number.value(),
                            value.x,
                            value.y
                        ));
                    }
                });
        });
    }
}

fn without_item_spacing<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let prev_item_spacing = ui.spacing_mut().item_spacing;
    ui.spacing_mut().item_spacing = egui::Vec2::new(prev_item_spacing.x, 0.0);
    let response = add_contents(ui);
    ui.spacing_mut().item_spacing = prev_item_spacing;
    response
}
