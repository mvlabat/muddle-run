use crate::{
    helpers::MouseEntityPicker, ui::MuddleInspectable, DelayServerTime, EstimatedServerTime,
    GameTicksPerSecond, TargetFramesAhead,
};
use bevy::{
    diagnostic::{DiagnosticMeasurement, Diagnostics, FrameTimeDiagnosticsPlugin},
    ecs::system::SystemParam,
    prelude::*,
    utils::HashMap,
};
use bevy_egui::{egui, egui::epaint::RectShape, EguiContext};
use mr_shared_lib::{
    client::components::DebugUiVisibility,
    framebuffer::FrameNumber,
    game::{
        client_factories::VisibilitySettings,
        components::{
            LevelObjectMovement, LevelObjectServerGhostChild, LevelObjectStaticGhostParent,
            PlayerDirection, Position,
        },
        level::LevelState,
    },
    messages::{EntityNetId, PlayerNetId},
    net::ConnectionState,
    player::Player,
    registry::EntityRegistry,
    GameState, SimulationTime,
};
use std::{collections::VecDeque, marker::PhantomData};

#[derive(SystemParam)]
pub struct DebugData<'w, 's> {
    game_state: Res<'w, State<GameState>>,
    time: Res<'w, SimulationTime>,
    current_ticks_per_second: Res<'w, GameTicksPerSecond>,
    delay_server_time: Res<'w, DelayServerTime>,
    target_frames_ahead: Res<'w, TargetFramesAhead>,
    estimated_server_time: Res<'w, EstimatedServerTime>,
    connection_state: Res<'w, ConnectionState>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

#[derive(Default)]
pub struct DebugUiState {
    pub show: bool,
    pub fps_history: VecDeque<DiagnosticMeasurement>,
    pub fps_history_len: usize,
    pub pause: bool,

    pub game_state: GameState,
    pub actual_frames_ahead: u16,
    pub target_frames_ahead: u16,
    pub current_ticks_per_second: f32,
    pub player_frame: FrameNumber,
    pub local_server_frame: FrameNumber,
    pub estimated_server_frame: FrameNumber,
    pub ahead_of_server: i32,
    pub delay_server_time: i16,
    pub rtt_millis: usize,
    pub packet_loss: f32,
    pub jitter_millis: usize,
}

pub fn update_debug_visibility(
    mut debug_ui_was_shown: Local<bool>,
    debug_ui_state: Res<DebugUiState>,
    mut visibility_settings: ResMut<VisibilitySettings>,
    mut debug_ui_visible: Query<&mut Visibility, With<DebugUiVisibility>>,
) {
    visibility_settings.debug = debug_ui_state.show;
    if *debug_ui_was_shown != debug_ui_state.show {
        for mut visible in debug_ui_visible.iter_mut() {
            visible.is_visible = debug_ui_state.show;
        }
    }
    *debug_ui_was_shown = debug_ui_state.show;
}

pub fn update_debug_ui_state(mut debug_ui_state: ResMut<DebugUiState>, debug_data: DebugData) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    if debug_ui_state.pause {
        return;
    }
    debug_ui_state.game_state = debug_data.game_state.current().clone();
    debug_ui_state.actual_frames_ahead = debug_data.time.player_frames_ahead();
    debug_ui_state.target_frames_ahead = debug_data.target_frames_ahead.target;
    debug_ui_state.current_ticks_per_second = debug_data.current_ticks_per_second.value;
    debug_ui_state.player_frame = debug_data.time.player_frame;
    debug_ui_state.local_server_frame = debug_data.time.server_frame;
    debug_ui_state.estimated_server_frame = debug_data.estimated_server_time.frame_number;
    debug_ui_state.delay_server_time = debug_data.delay_server_time.frame_count;
    debug_ui_state.rtt_millis = debug_data.connection_state.rtt_millis() as usize;
    debug_ui_state.packet_loss = debug_data.connection_state.packet_loss() * 100.0;
    debug_ui_state.jitter_millis = debug_data.connection_state.jitter_millis() as usize;
}

pub fn profiler_ui(
    // ResMut is intentional, to avoid fighting over the Mutex from different systems.
    mut egui_context: ResMut<EguiContext>,
    debug_ui_state: Res<DebugUiState>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let ctx = egui_context.ctx_mut();

    if !debug_ui_state.show {
        return;
    }

    egui::Window::new("Profiler")
        .default_size([1024.0, 600.0])
        .show(ctx, |_ui| {
            #[cfg(feature = "profiler")]
            puffin_egui::profiler_ui(_ui)
        });
}

pub fn debug_ui(
    // ResMut is intentional, to avoid fighting over the Mutex from different systems.
    mut egui_context: ResMut<EguiContext>,
    mut debug_ui_state: ResMut<DebugUiState>,
    diagnostics: Res<Diagnostics>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let ctx = egui_context.ctx_mut();

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

            ui.label(format!("Game state: {:?}", debug_ui_state.game_state));
            ui.label(format!(
                "Actual frames ahead: {}",
                debug_ui_state.actual_frames_ahead
            ));
            ui.label(format!(
                "Target frames ahead: {}",
                debug_ui_state.target_frames_ahead,
            ));
            ui.separator();
            ui.label(format!(
                "Tick rate: {} per second",
                debug_ui_state.current_ticks_per_second
            ));
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
                "Delay server time: {}",
                debug_ui_state.delay_server_time
            ));
            ui.separator();
            ui.label(format!("RTT: {}ms", debug_ui_state.rtt_millis));
            ui.label(format!("Packet loss: {:.2}%", debug_ui_state.packet_loss));
            ui.label(format!("Jitter: {}ms", debug_ui_state.jitter_millis));
        });
    }
}

#[derive(SystemParam)]
pub struct InspectObjectQueries<'w, 's> {
    players: Res<'w, HashMap<PlayerNetId, Player>>,
    player_registry: Res<'w, EntityRegistry<PlayerNetId>>,
    objects_registry: Res<'w, EntityRegistry<EntityNetId>>,
    level_state: Res<'w, LevelState>,
    positions: Query<'w, 's, &'static Position>,
    player_directions: Query<'w, 's, &'static PlayerDirection>,
    level_objects: Query<
        'w,
        's,
        (
            &'static LevelObjectMovement,
            Option<&'static LevelObjectServerGhostChild>,
        ),
    >,
    static_ghosts: Query<'w, 's, &'static LevelObjectStaticGhostParent>,
}

pub fn inspect_object(
    // ResMut is intentional, to avoid fighting over the Mutex from different systems.
    debug_ui_state: Res<DebugUiState>,
    mut egui_context: ResMut<EguiContext>,
    mut mouse_entity_picker: MouseEntityPicker<(), ()>,
    queries: InspectObjectQueries,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    if !debug_ui_state.show {
        return;
    }

    let ctx = egui_context.ctx_mut();
    if !ctx.is_pointer_over_area() {
        mouse_entity_picker.process_input(&mut None);
    }

    if let Some(mut entity) = mouse_entity_picker.picked_entity() {
        egui::Window::new("Inspect").show(ctx, |ui| {
            if let Some(player_name) = queries
                .player_registry
                .get_id(entity)
                .and_then(|player_net_id| queries.players.get(&player_net_id))
                .map(|player| player.nickname.clone())
            {
                ui.label(format!("Player name: {}", player_name));
            }
            ui.label(format!("Entity: {:?}", entity));
            if let Ok(LevelObjectStaticGhostParent(parent_entity)) =
                queries.static_ghosts.get(entity)
            {
                entity = *parent_entity;
            }
            if let Some(level_object_label) = queries
                .objects_registry
                .get_id(entity)
                .and_then(|object_net_id| queries.level_state.objects.get(&object_net_id))
                .map(|level_object| level_object.label.clone())
            {
                ui.label(&level_object_label);
            }
            if let Ok((level_object_movement, level_object_server_ghost)) =
                queries.level_objects.get(entity)
            {
                if let Some(LevelObjectServerGhostChild(server_ghost_entity)) =
                    level_object_server_ghost
                {
                    ui.label(format!("Server ghost: {:?}", server_ghost_entity));
                }
                ui.collapsing("Route", |ui| {
                    ui.label(format!(
                        "Frame started: {}",
                        level_object_movement.frame_started
                    ));
                    ui.label(format!("Init vec: {}", level_object_movement.init_vec));
                    ui.label(format!("Period: {}", level_object_movement.period));
                });
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
    let size = vec2(ui.max_rect().width(), height);
    let (rect, response) = ui.allocate_at_least(size, Sense::hover());
    let style = ui.style().noninteractive();

    let mut shapes = vec![Shape::Rect(RectShape {
        rect,
        rounding: style.rounding,
        fill: ui.style().visuals.extreme_bg_color,
        stroke: ui.style().noninteractive().bg_stroke,
    })];

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
                &ui.fonts(),
                pos2(rect.left(), y),
                egui::Align2::LEFT_BOTTOM,
                text,
                TextStyle::Monospace.resolve(ui.style()),
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
