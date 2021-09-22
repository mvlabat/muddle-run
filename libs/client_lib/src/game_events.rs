use bevy::ecs::system::{Query, Res};
use mr_shared_lib::{
    framebuffer::FrameNumber, game::components::Spawned, player::PlayerSystemParamsMut,
    simulations_per_second, util::player_respawn_time, SimulationTime,
};

pub fn process_scheduled_spawns(
    time: Res<SimulationTime>,
    players: PlayerSystemParamsMut,
    players_query: Query<&Spawned>,
) {
    let PlayerSystemParamsMut {
        mut players,
        player_registry,
    } = players;
    let iter = players.iter_mut().filter_map(move |(net_id, player)| {
        player_registry
            .get_entity(*net_id)
            .map(|entity| (entity, player))
    });

    for (entity, player) in iter {
        if let Ok(spawned) = players_query.get(entity) {
            if !spawned.is_spawned(time.player_frame) {
                continue;
            }
        } else {
            continue;
        }

        if let Some((respawning_at, _)) = player.respawning_at {
            // A kludge to avoid `respawning_at` disappear immediately.
            // TODO: Probably, there's a better way to do this.
            if time.player_frame
                > respawning_at - player_respawn_time() + FrameNumber::new(simulations_per_second())
            {
                player.respawning_at = None;
            }
        }
    }
}
