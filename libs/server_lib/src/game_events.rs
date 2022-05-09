use bevy::{
    ecs::{
        event::EventReader,
        system::{Res, ResMut},
    },
    utils::HashMap,
};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands,
        commands::{DeferredQueue, DespawnPlayer},
        events::{PlayerDeath, PlayerFinish},
    },
    messages::{DeferredMessagesQueue, PlayerNetId, RespawnPlayer, RespawnPlayerReason},
    player::{Player, PlayerSystemParamsMut},
    server::level_spawn_location_service::LevelSpawnLocationService,
    util::PLAYER_RESPAWN_TIME,
    SimulationTime,
};

pub fn process_player_events(
    time: Res<SimulationTime>,
    mut player_finish_events: EventReader<PlayerFinish>,
    mut player_death_events: EventReader<PlayerDeath>,
    mut player_params: PlayerSystemParamsMut,
    mut respawn_player_messages_queue: ResMut<DeferredMessagesQueue<RespawnPlayer>>,
    mut despawn_players_commands: ResMut<DeferredQueue<commands::DespawnPlayer>>,
) {
    let respawn_at = time.server_frame + PLAYER_RESPAWN_TIME;

    let mut respawns = Vec::new();
    respawns.extend(
        player_finish_events
            .iter()
            .map(|PlayerFinish(player_entity)| (player_entity, RespawnPlayerReason::Finish)),
    );
    respawns.extend(
        player_death_events
            .iter()
            .map(|PlayerDeath(player_entity)| (player_entity, RespawnPlayerReason::Death)),
    );

    for (player_entity, reason) in respawns.into_iter() {
        let net_id = player_params
            .player_registry
            .get_id(*player_entity)
            .expect("Expected a registered player for a Finish event");

        let player = player_params
            .players
            .get_mut(&net_id)
            .expect("Expected a registered player for a Finish event");
        player.respawning_at = Some((respawn_at, reason));
        match reason {
            RespawnPlayerReason::Finish => {
                player.finishes += 1;
            }
            RespawnPlayerReason::Death => {
                player.deaths += 1;
            }
        }

        respawn_player_messages_queue.push(RespawnPlayer {
            net_id,
            reason,
            frame_number: respawn_at,
        });
        despawn_players_commands.push(DespawnPlayer {
            net_id,
            frame_number: time.server_frame + FrameNumber::new(1),
        })
    }
}

pub fn process_scheduled_spawns(
    time: Res<SimulationTime>,
    level_spawn_location_service: LevelSpawnLocationService,
    mut spawn_players_commands: ResMut<DeferredQueue<commands::SpawnPlayer>>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
) {
    for (player_net_id, player) in players.iter_mut() {
        if let Some((spawn_at, _)) = player.respawning_at {
            if time.server_frame >= spawn_at {
                spawn_players_commands.push(commands::SpawnPlayer {
                    net_id: *player_net_id,
                    start_position: level_spawn_location_service.spawn_position(time.server_frame),
                    is_player_frame_simulated: false,
                });
                player.respawning_at = None;
            }
        }
    }
}
