use crate::{input::MouseRay, ui::MuddleInspectable};
use bevy::{
    diagnostic::{DiagnosticMeasurement, Diagnostics, FrameTimeDiagnosticsPlugin},
    ecs::SystemParam,
    prelude::*,
};
use bevy_egui::{egui, EguiContext, EguiSettings};
use bevy_rapier3d::{
    physics::ColliderHandleComponent,
    rapier::{
        geometry::{ColliderSet, InteractionGroups},
        pipeline::QueryPipeline,
    },
};
use mr_shared_lib::{
    game::components::{PlayerDirection, Position},
    messages::PlayerNetId,
    player::Player,
    registry::EntityRegistry,
};
use std::collections::{HashMap, VecDeque};

pub fn update_ui_scale_factor(mut egui_settings: ResMut<EguiSettings>, windows: Res<Windows>) {
    if let Some(window) = windows.get_primary() {
        egui_settings.scale_factor = 1.0 / window.scale_factor();
    }
}

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

pub struct InspectableObject {
    entity: Option<Entity>,
}

impl Default for InspectableObject {
    fn default() -> Self {
        Self { entity: None }
    }
}

#[derive(SystemParam)]
pub struct InspectObjectQueries<'a> {
    players: Res<'a, HashMap<PlayerNetId, Player>>,
    player_registry: Res<'a, EntityRegistry<PlayerNetId>>,
    colliders: Query<'a, (Entity, &'a ColliderHandleComponent)>,
    positions: Query<'a, (Entity, &'a Position)>,
    player_directions: Query<'a, (Entity, &'a PlayerDirection)>,
}

pub fn inspect_object(
    mut egui_context: ResMut<EguiContext>,
    mut inspectable_object: Local<InspectableObject>,
    mouse_input: Res<Input<MouseButton>>,
    mouse_ray: Res<MouseRay>,
    query_pipeline: Res<QueryPipeline>,
    collider_set: Res<ColliderSet>,
    queries: InspectObjectQueries,
) {
    let ctx = &mut egui_context.ctx;
    if mouse_input.just_pressed(MouseButton::Left) && !ctx.is_mouse_over_area() {
        if let Some((collider, _)) = query_pipeline.cast_ray(
            &collider_set,
            &mouse_ray.0,
            f32::MAX,
            true,
            InteractionGroups::all(),
        ) {
            let (entity, _) = queries
                .colliders
                .iter()
                .find(|(_, collider_component)| collider_component.handle() == collider)
                .unwrap();

            inspectable_object.entity = Some(entity);
        } else {
            inspectable_object.entity = None;
        }
    }

    if let Some(entity) = inspectable_object.entity {
        egui::Window::new("Inspect").show(ctx, |ui| {
            if let Some(player_name) = queries
                .player_registry
                .get_id(entity)
                .and_then(|player_net_id| queries.players.get(&player_net_id))
                .map(|player| player.nickname.clone())
            {
                ui.label(format!("Player name: {}", player_name));
            }
            if let Ok((_, position)) = queries.positions.get(entity) {
                position.inspect(ui);
            }
            if let Ok((_, player_direction)) = queries.player_directions.get(entity) {
                player_direction.inspect(ui);
            }
        });
    }
}

fn graph(
    ui: &mut egui::Ui,
    history: &VecDeque<DiagnosticMeasurement>,
    max_len: usize,
) -> egui::Response {
    use egui::*;

    let graph_top_value = 720.0;

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
