use crate::{input::LevelObjectRequestsQueue, CurrentPlayerNetId, LevelObjectCorrelations};
use bevy::ecs::system::{Res, ResMut};
use bevy_egui::{egui, EguiContext};
use mr_shared_lib::{
    game::{level::LevelObjectDesc, level_objects::PlaneDesc},
    messages::{PlayerNetId, SpawnLevelObjectRequest, SpawnLevelObjectRequestBody},
    player::{Player, PlayerRole},
};
use std::collections::HashMap;

pub fn builder_ui(
    egui_context: ResMut<EguiContext>,
    current_player_net_id: Res<CurrentPlayerNetId>,
    players: Res<HashMap<PlayerNetId, Player>>,
    mut level_object_correlations: ResMut<LevelObjectCorrelations>,
    mut level_object_requests: ResMut<LevelObjectRequestsQueue>,
) {
    let current_player_id = match current_player_net_id.0 {
        Some(current_player_id) => current_player_id,
        None => return,
    };
    let player = players
        .get(&current_player_id)
        .expect("Expected a current player to exist");
    if !matches!(player.role, PlayerRole::Builder) {
        return;
    }

    let ctx = egui_context.ctx();
    egui::Window::new("Builder menu").show(ctx, |ui| {
        if ui.button("Plane").clicked() {
            let correlation_id = level_object_correlations.next_correlation_id();
            level_object_requests
                .spawn_requests
                .push(SpawnLevelObjectRequest {
                    correlation_id,
                    body: SpawnLevelObjectRequestBody::New(LevelObjectDesc::Plane(PlaneDesc {
                        size: 50.0,
                    })),
                })
        }
    });
}
