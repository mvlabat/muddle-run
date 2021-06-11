use crate::{
    input::MouseRay, ui::MuddleInspectable, AdjustedSpeedReason, EstimatedServerTime,
    GameTicksPerSecond, PlayerDelay, TargetFramesAhead,
};
use bevy::{
    diagnostic::{DiagnosticMeasurement, Diagnostics, FrameTimeDiagnosticsPlugin},
    ecs::system::SystemParam,
    log,
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
    framebuffer::FrameNumber,
    game::components::{LevelObjectLabel, PlayerDirection, Position},
    messages::PlayerNetId,
    net::ConnectionState,
    player::Player,
    registry::EntityRegistry,
    SimulationTime,
};
use std::collections::{HashMap, VecDeque};

pub fn update_ui_scale_factor(mut egui_settings: ResMut<EguiSettings>, windows: Res<Windows>) {
    if let Some(window) = windows.get_primary() {
        egui_settings.scale_factor = 1.0 / window.scale_factor();
    }
}

#[derive(SystemParam)]
pub struct DebugData<'a> {
    time: Res<'a, SimulationTime>,
    tick_rate: Res<'a, GameTicksPerSecond>,
    player_delay: Res<'a, PlayerDelay>,
    adjusted_speed_reason: Res<'a, AdjustedSpeedReason>,
    target_frames_ahead: Res<'a, TargetFramesAhead>,
    estimated_server_time: Res<'a, EstimatedServerTime>,
    connection_state: Res<'a, ConnectionState>,
}

#[derive(Default)]
pub struct DebugUiState {
    pub show: bool,
    pub fps_history: VecDeque<DiagnosticMeasurement>,
    pub fps_history_len: usize,
    pub pause: bool,

    pub frames_ahead: FrameNumber,
    pub target_frames_ahead: FrameNumber,
    pub tick_rate: u16,
    pub player_frame: FrameNumber,
    pub local_server_frame: FrameNumber,
    pub estimated_server_frame: FrameNumber,
    pub ahead_of_server: i32,
    pub player_delay: i16,
    pub adjusted_speed_reason: AdjustedSpeedReason,
    pub rtt_millis: usize,
    pub packet_loss: f32,
    pub jitter_millis: usize,
}

pub fn update_debug_ui_state(mut debug_ui_state: ResMut<DebugUiState>, debug_data: DebugData) {
    if debug_ui_state.pause {
        return;
    }
    debug_ui_state.frames_ahead = debug_data.time.player_frame - debug_data.time.server_frame;
    debug_ui_state.target_frames_ahead = debug_data.target_frames_ahead.frames_count;
    debug_ui_state.tick_rate = debug_data.tick_rate.rate;
    debug_ui_state.player_frame = debug_data.time.player_frame;
    debug_ui_state.local_server_frame = debug_data.time.server_frame;
    debug_ui_state.estimated_server_frame = debug_data.estimated_server_time.frame_number;
    debug_ui_state.ahead_of_server = debug_data.time.player_frame.value() as i32
        - debug_data.estimated_server_time.frame_number.value() as i32
        + (debug_data.time.player_frame - debug_data.estimated_server_time.updated_at).value()
            as i32;
    debug_ui_state.player_delay = debug_data.player_delay.frame_count;
    debug_ui_state.adjusted_speed_reason = *debug_data.adjusted_speed_reason;
    debug_ui_state.rtt_millis = debug_data.connection_state.rtt_millis() as usize;
    debug_ui_state.packet_loss = debug_data.connection_state.packet_loss() * 100.0;
    debug_ui_state.jitter_millis = debug_data.connection_state.jitter_millis() as usize;
}

pub fn debug_ui(
    // ResMut is intentional, to avoid fighting over the Mutex from different systems.
    egui_context: ResMut<EguiContext>,
    mut debug_ui_state: ResMut<DebugUiState>,
    diagnostics: Res<Diagnostics>,
) {
    let ctx = egui_context.ctx();

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

            ui.separator();
            if debug_ui_state.pause {
                if ui.button("Unpause").clicked() {
                    debug_ui_state.pause = false;
                }
            } else if ui.button("Pause").clicked() {
                debug_ui_state.pause = true;
            }

            ui.label(format!("Frames ahead: {}", debug_ui_state.frames_ahead,));
            ui.label(format!(
                "Target frames ahead: {}",
                debug_ui_state.target_frames_ahead,
            ));
            ui.separator();
            ui.label(format!("Tick rate: {}", debug_ui_state.tick_rate));
            ui.label(format!("Player frame: {}", debug_ui_state.player_frame));
            ui.label(format!(
                "Local server frame: {}",
                debug_ui_state.local_server_frame
            ));
            ui.label(format!(
                "Estimated server frame: {}",
                debug_ui_state.estimated_server_frame
            ));
            ui.label(format!(
                "Ahead of server: {}",
                debug_ui_state.ahead_of_server
            ));
            ui.label(format!("Player delay: {}", debug_ui_state.player_delay));
            ui.label(format!("{:?}", debug_ui_state.adjusted_speed_reason));
            ui.separator();
            ui.label(format!("RTT: {}ms", debug_ui_state.rtt_millis));
            ui.label(format!("Packet loss: {:.2}%", debug_ui_state.packet_loss));
            ui.label(format!("Jitter: {}ms", debug_ui_state.jitter_millis));
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
    colliders: Query<'a, (Entity, &'static ColliderHandleComponent)>,
    positions: Query<'a, &'static Position>,
    player_directions: Query<'a, &'static PlayerDirection>,
    level_object_labels: Query<'a, &'static LevelObjectLabel>,
}

pub fn inspect_object(
    // ResMut is intentional, to avoid fighting over the Mutex from different systems.
    egui_context: ResMut<EguiContext>,
    mut inspectable_object: Local<InspectableObject>,
    mouse_input: Res<Input<MouseButton>>,
    mouse_ray: Res<MouseRay>,
    query_pipeline: Res<QueryPipeline>,
    collider_set: Res<ColliderSet>,
    queries: InspectObjectQueries,
) {
    let ctx = egui_context.ctx();
    if mouse_input.just_pressed(MouseButton::Left) && !ctx.is_pointer_over_area() {
        if let Some((collider, _)) = query_pipeline.cast_ray(
            &collider_set,
            &mouse_ray.0,
            f32::MAX,
            true,
            InteractionGroups::all(),
            None,
        ) {
            if let Some((entity, _)) = queries
                .colliders
                .iter()
                .find(|(_, collider_component)| collider_component.handle() == collider)
            {
                inspectable_object.entity = Some(entity);
            } else {
                log::error!("No entity with collider {:?} was found", collider);
            }
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
            if let Ok(level_object_label) = queries.level_object_labels.get(entity) {
                ui.label(&level_object_label.0);
            }
            if let Ok(position) = queries.positions.get(entity) {
                position.inspect(ui);
            }
            if let Ok(player_direction) = queries.player_directions.get(entity) {
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
        fill: ui.style().visuals.extreme_bg_color,
        stroke: ui.style().noninteractive().bg_stroke,
    }];

    let rect = rect.shrink(4.0);
    let line_stroke = Stroke::new(1.0, Color32::from_additive_luminance(128));

    if let Some(mouse_pos) = ui.input().pointer.hover_pos() {
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
