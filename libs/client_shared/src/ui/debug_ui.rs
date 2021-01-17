use bevy::{
    diagnostic::{DiagnosticMeasurement, Diagnostics, FrameTimeDiagnosticsPlugin},
    prelude::*,
};
use bevy_egui::{egui, EguiContext};
use std::collections::VecDeque;

#[derive(Default)]
pub struct DebugUiState {
    pub show: bool,
    pub fps_history: VecDeque<DiagnosticMeasurement>,
    pub fps_history_len: usize,
}

pub fn debug_ui(
    mut egui_context: ResMut<EguiContext>,
    mut debug_ui_state: ResMut<DebugUiState>,
    diagnostics: Res<Diagnostics>,
) {
    let ctx = &mut egui_context.ctx;

    if let Some(fps_diagnostic) = diagnostics.get(FrameTimeDiagnosticsPlugin::FPS) {
        debug_ui_state.fps_history_len = fps_diagnostic.get_max_history_length();
    }
    if let Some(measurement) = diagnostics.get_measurement(FrameTimeDiagnosticsPlugin::FPS) {
        if debug_ui_state.fps_history.len() == debug_ui_state.fps_history_len {
            debug_ui_state.fps_history.pop_front();
        }
        debug_ui_state.fps_history.push_back(DiagnosticMeasurement {
            time: measurement.time,
            value: measurement.value,
        });
    }

    if debug_ui_state.show {
        egui::Window::new("Debug").show(ctx, |ui| {
            egui::CollapsingHeader::new("📊 FPS graph")
                .default_open(false)
                .show(ui, |ui| {
                    graph(
                        ui,
                        &debug_ui_state.fps_history,
                        debug_ui_state.fps_history_len,
                    );
                });
        });
    }
}

fn graph(
    ui: &mut egui::Ui,
    history: &VecDeque<DiagnosticMeasurement>,
    max_len: usize,
) -> egui::Response {
    use egui::*;

    let graph_top_value = 360.0;

    // TODO (from Egui): we should not use `slider_width` as default graph width.
    let height = ui.style().spacing.slider_width;
    let size = vec2(ui.available_size_before_wrap_finite().x, height);
    let (rect, response) = ui.allocate_at_least(size, Sense::hover());
    let style = ui.style().noninteractive();

    let mut shapes = vec![Shape::Rect {
        rect,
        corner_radius: style.corner_radius,
        fill: ui.style().visuals.dark_bg_color,
        stroke: ui.style().noninteractive().bg_stroke,
    }];

    let rect = rect.shrink(4.0);
    let line_stroke = Stroke::new(1.0, Color32::from_additive_luminance(128));

    if let Some(mouse_pos) = ui.input().mouse.pos {
        if rect.contains(mouse_pos) {
            let y = mouse_pos.y;
            shapes.push(Shape::line_segment(
                [pos2(rect.left(), y), pos2(rect.right(), y)],
                line_stroke,
            ));
            let value = remap(y, rect.bottom_up_range(), 0.0..=graph_top_value);
            let text = format!("{:.1}", value);
            shapes.push(Shape::text(
                ui.fonts(),
                pos2(rect.left(), y),
                egui::Align2::LEFT_BOTTOM,
                text,
                TextStyle::Monospace,
                Color32::WHITE,
            ));
        }
    }

    let circle_color = Color32::from_additive_luminance(196);
    let radius = 2.0;

    for (i, DiagnosticMeasurement { value, .. }) in history.iter().enumerate() {
        let value = *value as f32;
        let age = i as f32;
        let x = remap(age, 0.0..=max_len as f32, rect.x_range());
        let y = remap_clamp(value, 0.0..=graph_top_value, rect.bottom_up_range());

        shapes.push(Shape::line_segment(
            [pos2(x, rect.bottom()), egui::pos2(x, y)],
            line_stroke,
        ));

        if value < graph_top_value {
            shapes.push(Shape::circle_filled(pos2(x, y), radius, circle_color));
        }
    }

    ui.painter().extend(shapes);

    response
}
