use crate::{
    components::{CameraPivotDirection, CameraPivotTag},
    CurrentPlayerNetId, MainCameraPivotEntity,
};
use bevy::{
    core::Time,
    ecs::{
        entity::Entity,
        query::{Changed, With},
        system::{Commands, Query, QuerySet, RemovedComponents, Res},
    },
    log,
    transform::components::{Parent, Transform},
};
use mr_shared_lib::{
    game::components::{PlayerTag, Position, Spawned},
    messages::PlayerNetId,
    registry::EntityRegistry,
    GameTime, PLAYER_RADIUS,
};

const CAMERA_MOVEMENT_SPEED: f32 = 4.0;

pub type ReattachCameraQueries<'a, 'b, 'c> = QuerySet<(
    Query<'a, Option<&'b Parent>, With<CameraPivotTag>>,
    Query<'a, (Entity, &'c Spawned, &'c Position), (Changed<Spawned>, With<PlayerTag>)>,
)>;

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
    let camera_parent = queries
        .q0()
        .get(main_camera_pivot.0)
        .expect("Expected the camera to initialize in `basic_scene`");

    let mut main_camera_pivot_commands = commands.entity(main_camera_pivot.0);
    // TODO: track the following (to avoid iterating each frame):
    //  https://github.com/bevyengine/bevy/pull/2330#issuecomment-861605604
    //  https://github.com/bevyengine/bevy/issues/2348
    for (player_entity, spawned, position) in queries.q1().iter() {
        let position = position.buffer.last().cloned().unwrap_or_default();
        let is_current_player = current_player_net_id.0.map_or(false, |player_net_id| {
            Some(player_net_id) == player_registry.get_id(player_entity)
        });
        if is_current_player {
            match (
                spawned.is_spawned(time.frame_number),
                camera_parent.is_some(),
            ) {
                (true, false) => {
                    log::trace!("Attaching camera pivot to a player");
                    main_camera_pivot_commands
                        .insert(Parent(player_entity))
                        .insert(Transform::from_xyz(0.0, 0.0, -PLAYER_RADIUS));
                }
                (false, true) => {
                    log::trace!("Freeing camera pivot");
                    main_camera_pivot_commands
                        .remove::<Parent>()
                        .insert(Transform::from_xyz(position.x, position.y, 0.0));
                }
                _ => {}
            }
        }
    }

    // Usually, this is not needed. But we'll have this clean up just in case if we didn't catch
    // the despawn event from the `Spawned` component change.
    for despawned_player_entity in despawned_player_events.iter() {
        let is_current_player = current_player_net_id.0.map_or(false, |player_net_id| {
            Some(player_net_id) == player_registry.get_id(despawned_player_entity)
        });
        if camera_parent.is_some() {
            log::warn!("Resetting camera pivot didn't happen in time, resetting camera position");
            if is_current_player {
                main_camera_pivot_commands
                    .remove::<Parent>()
                    .insert(Transform::from_xyz(0.0, 0.0, 0.0));
            }
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
