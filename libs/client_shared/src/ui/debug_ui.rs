use bevy::prelude::*;
use bevy_egui::{egui, EguiContext};

#[derive(Default)]
pub struct DebugUiState {
    pub show: bool,
}

pub fn debug_ui(mut egui_context: ResMut<EguiContext>, debug_ui_state: Res<DebugUiState>) {
    let ctx = &mut egui_context.ctx;

    if debug_ui_state.show {
        egui::Window::new("Debug").show(ctx, |_ui| {});
    }
}
