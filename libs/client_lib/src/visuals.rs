use crate::helpers::PlayerParams;
use bevy::{
    ecs::{
        entity::Entity,
        query::With,
        system::{Local, Query, QuerySet, ResMut},
    },
    render::draw::Visible,
};
use mr_shared_lib::{
    game::{
        client_factories::VisibilitySettings,
        components::{LevelObjectStaticGhost, LevelObjectTag},
        level::{LevelObjectDesc, LevelParams},
    },
    player::PlayerRole,
};

pub type Queries<'a, 'b, 'c> = QuerySet<(
    Query<'a, (Entity, &'b mut Visible), With<LevelObjectTag>>,
    Query<'a, &'c mut Visible, With<LevelObjectStaticGhost>>,
)>;

pub fn control_builder_visibility(
    mut prev_role: Local<Option<PlayerRole>>,
    player_params: PlayerParams,
    level_params: LevelParams,
    mut visibility_settings: ResMut<VisibilitySettings>,
    mut queries: Queries,
) {
    if let Some(player) = player_params.current_player() {
        if *prev_role == Some(player.role) {
            return;
        }
        *prev_role = Some(player.role);

        let is_builder = match player.role {
            PlayerRole::Runner => false,
            PlayerRole::Builder => true,
        };
        visibility_settings.route_points = is_builder;
        visibility_settings.ghosts = is_builder;

        let level_objects_query = queries.q0_mut();
        for (entity, mut visible) in level_objects_query.iter_mut() {
            if let Some(level_object) = level_params.level_object_by_entity(entity) {
                match level_object.desc {
                    LevelObjectDesc::RoutePoint(_) => {
                        visible.is_visible = is_builder;
                    }
                    LevelObjectDesc::Plane(_) | LevelObjectDesc::Cube(_) => {}
                }
            }
        }
        let ghosts_query = queries.q1_mut();
        for mut visible in ghosts_query.iter_mut() {
            visible.is_visible = is_builder;
        }
    }
}
