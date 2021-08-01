use crate::helpers::PlayerParams;
use bevy::{
    ecs::{
        entity::Entity,
        query::With,
        system::{Local, Query, ResMut},
    },
    prelude::Without,
    render::draw::Visible,
    transform::components::Transform,
};
use mr_shared_lib::{
    game::{
        client_factories::VisibilitySettings,
        components::{LevelObjectStaticGhost, LevelObjectTag},
        level::{LevelObjectDesc, LevelParams},
    },
    player::PlayerRole,
};

pub fn control_builder_visibility(
    mut prev_role: Local<Option<PlayerRole>>,
    player_params: PlayerParams,
    level_params: LevelParams,
    mut visibility_settings: ResMut<VisibilitySettings>,
    mut level_objects_query: Query<(Entity, &Transform, &mut Visible), With<LevelObjectTag>>,
    mut ghosts_query: Query<
        (&Transform, &mut Visible, &LevelObjectStaticGhost),
        Without<LevelObjectTag>,
    >,
) {
    puffin::profile_function!();
    if let Some(player) = player_params.current_player() {
        let is_builder = match player.role {
            PlayerRole::Runner => false,
            PlayerRole::Builder => true,
        };
        visibility_settings.route_points = is_builder;
        visibility_settings.ghosts = is_builder;

        // These change only on role update, there's no other reason to update them.
        if *prev_role != Some(player.role) {
            for (entity, _, mut visible) in level_objects_query.iter_mut() {
                if let Some(level_object) = level_params.level_object_by_entity(entity) {
                    match level_object.desc {
                        LevelObjectDesc::RoutePoint(_) => {
                            visible.is_visible = is_builder;
                        }
                        LevelObjectDesc::Plane(_) | LevelObjectDesc::Cube(_) => {}
                    }
                }
            }
        }

        for (transform, mut visible, LevelObjectStaticGhost(parent_entity)) in
            ghosts_query.iter_mut()
        {
            let parent_transform = level_objects_query
                .get_component::<Transform>(*parent_entity)
                .unwrap();
            if (transform.translation.x - parent_transform.translation.x).abs() < f32::EPSILON
                && (transform.translation.y - parent_transform.translation.y).abs() < f32::EPSILON
            {
                visible.is_visible = false
            } else {
                visible.is_visible = is_builder;
            }
        }

        *prev_role = Some(player.role);
    }
}
