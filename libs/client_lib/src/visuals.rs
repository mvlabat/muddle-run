use crate::helpers::PlayerParams;
use bevy::{
    ecs::{
        entity::Entity,
        query::With,
        system::{Local, Query, ResMut},
    },
    render::draw::Visible,
};
use mr_shared_lib::{
    game::{
        client_factories::VisibilitySettings,
        components::LevelObjectTag,
        level::{LevelObjectDesc, LevelParams},
    },
    player::PlayerRole,
};

pub fn control_builder_visibility(
    mut prev_role: Local<Option<PlayerRole>>,
    player_params: PlayerParams,
    level_params: LevelParams,
    mut visibility_settings: ResMut<VisibilitySettings>,
    mut level_objects_query: Query<(Entity, &mut Visible), With<LevelObjectTag>>,
) {
    if let Some(player) = player_params.current_player() {
        if *prev_role == Some(player.role) {
            return;
        }
        *prev_role = Some(player.role);

        let pivots_should_be_visible = match player.role {
            PlayerRole::Runner => false,
            PlayerRole::Builder => true,
        };
        visibility_settings.pivot_points = pivots_should_be_visible;
        for (entity, mut visible) in level_objects_query.iter_mut() {
            if let Some(level_object) = level_params.level_object_by_entity(entity) {
                match level_object.desc {
                    LevelObjectDesc::PivotPoint(_) => {
                        visible.is_visible = pivots_should_be_visible;
                    }
                    LevelObjectDesc::Plane(_) | LevelObjectDesc::Cube(_) => {}
                }
            }
        }
    }
}
