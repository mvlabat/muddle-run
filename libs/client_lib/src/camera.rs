use crate::{
    components::{CameraPivotDirection, CameraPivotTag},
    CurrentPlayerNetId, MainCameraPivotEntity,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::{Changed, With},
        system::{Commands, Query, RemovedComponents, Res, SystemParam},
    },
    hierarchy::{BuildChildren, Parent},
    log,
    time::Time,
    transform::components::Transform,
};
use mr_shared_lib::{
    game::components::{PlayerTag, Position, Spawned},
    messages::PlayerNetId,
    registry::EntityRegistry,
    GameTime, PLAYER_RADIUS,
};

const CAMERA_MOVEMENT_SPEED: f32 = 4.0;

pub type SpawnedOrDespawnedPlayers<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static Spawned, &'static Position),
    (Changed<Spawned>, With<PlayerTag>),
>;

#[derive(SystemParam)]
pub struct ReattachCameraQueries<'w, 's> {
    camera_pivot_parents: Query<'w, 's, Option<&'static Parent>, With<CameraPivotTag>>,
    spawned_or_despawned_players: SpawnedOrDespawnedPlayers<'w, 's>,
    all_entities: Query<'w, 's, Entity>,
}

pub fn reattach_camera(
    mut commands: Commands,
    time: Res<GameTime>,
    main_camera_pivot: Res<MainCameraPivotEntity>,
    current_player_net_id: Res<CurrentPlayerNetId>,
    player_registry: Res<EntityRegistry<PlayerNetId>>,
    despawned_player_events: RemovedComponents<PlayerTag>,
    queries: ReattachCameraQueries,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let camera_pivot_parent = queries
        .camera_pivot_parents
        .get(main_camera_pivot.0)
        .expect("Expected the camera to initialize in `basic_scene`");

    let mut main_camera_pivot_commands = commands.entity(main_camera_pivot.0);
    // TODO: track the following (to avoid iterating each frame):
    //  https://github.com/bevyengine/bevy/pull/2330#issuecomment-861605604
    //  https://github.com/bevyengine/bevy/issues/2348
    for (player_entity, spawned, position) in queries.spawned_or_despawned_players.iter() {
        let position = position.buffer.last().cloned().unwrap_or_default();
        let is_current_player = current_player_net_id.0.map_or(false, |player_net_id| {
            Some(player_net_id) == player_registry.get_id(player_entity)
        });
        if is_current_player {
            match (
                spawned.is_spawned(time.frame_number),
                camera_pivot_parent.is_some(),
            ) {
                (true, false) => {
                    log::debug!("Attaching camera pivot to a player");
                    main_camera_pivot_commands.insert(Transform::from_xyz(
                        0.0,
                        0.0,
                        -PLAYER_RADIUS,
                    ));
                    main_camera_pivot_commands.set_parent(player_entity);
                }
                (false, true) => {
                    log::debug!("Freeing camera pivot");
                    main_camera_pivot_commands
                        .insert(Transform::from_xyz(position.x, position.y, 0.0))
                        .remove_parent();
                }
                _ => {}
            }
        }
    }

    // Usually, this is not needed. But we'll have this clean up just in case if we
    // didn't catch the despawn event from the `Spawned` component change.
    let mut failed_to_deattach = false;
    for despawned_player_entity in despawned_player_events.iter() {
        let is_current_player = current_player_net_id.0.map_or(false, |player_net_id| {
            Some(player_net_id) == player_registry.get_id(despawned_player_entity)
        });
        if camera_pivot_parent.is_some() && is_current_player {
            failed_to_deattach = true;
            log::warn!("Resetting camera pivot didn't happen in time, resetting camera position");
        }
    }

    // If camera has parent, but the current player doesn't exist, it most likely
    // was caused by the game restart. This is a valid scenario so we don't emit
    // warnings unlike with `failed_to_deattach`.
    if let (Some(camera_pivot_parent), true) = (
        camera_pivot_parent,
        (failed_to_deattach || current_player_net_id.0.is_none()),
    ) {
        log::debug!("Freeing camera pivot");
        main_camera_pivot_commands.insert(Transform::from_xyz(0.0, 0.0, 0.0));
        // If an entity was removed with `World::despawn` (that's what happens when we
        // restart the game), calling `remove_parent` will panic.
        if queries.all_entities.contains(camera_pivot_parent.get()) {
            main_camera_pivot_commands.remove_parent();
        } else {
            // We can safely just remove the component since the parent no longer exists.
            main_camera_pivot_commands.remove::<Parent>();
        }
    }
}

pub fn move_free_camera_pivot(
    time: Res<Time>,
    main_camera_pivot: Res<MainCameraPivotEntity>,
    mut camera_pivot_query: Query<(&CameraPivotDirection, &mut Transform)>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let (direction, mut transform) = camera_pivot_query
        .get_mut(main_camera_pivot.0)
        .expect("Expected the camera to initialize in `basic_scene`");
    let d = direction.0.normalize_or_zero() * CAMERA_MOVEMENT_SPEED * time.delta_seconds();
    transform.translation.x += d.x;
    transform.translation.y += d.y;
}
