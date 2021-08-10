use crate::{
    components::LevelObjectControlPoint,
    helpers::PlayerParams,
    input::LevelObjectRequestsQueue,
    ui::builder_ui::{EditedLevelObject, MouseInput},
};
use bevy::{
    asset::Handle,
    ecs::{
        entity::Entity,
        query::With,
        system::{Commands, Local, Query, Res, ResMut, SystemParam},
    },
    input::mouse::MouseButton,
    math::{Vec2, Vec3Swizzles},
    pbr::PbrBundle,
    prelude::{StandardMaterial, Without},
    render::draw::Visible,
    transform::components::{Children, Parent, Transform},
};
use mr_shared_lib::{
    client::{assets::MuddleAssets, components::LevelObjectControlPoints},
    game::{
        client_factories::VisibilitySettings,
        components::{LevelObjectStaticGhost, LevelObjectStaticGhostParent, LevelObjectTag},
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

pub fn spawn_control_points(
    mut commands: Commands,
    muddle_assets: MuddleAssets,
    edited_level_object: Res<EditedLevelObject>,
    mut prev_edited_level_object: Local<Option<Entity>>,
    mut control_points_parent_query: Query<&LevelObjectStaticGhostParent>,
    control_points_query: Query<Entity, With<LevelObjectControlPoint>>,
) {
    let edited_level_object_entity = edited_level_object
        .object
        .as_ref()
        .map(|(entity, _)| *entity);
    let mut prev_edited_level_object_entity = None;
    let changed = if *prev_edited_level_object != edited_level_object_entity {
        prev_edited_level_object_entity = *prev_edited_level_object;
        *prev_edited_level_object = edited_level_object_entity;
        true
    } else {
        false
    };

    if let Some((edited_level_object_entity, edited_level_object)) = &edited_level_object.object {
        if changed {
            if let Ok(LevelObjectStaticGhostParent(ghost_entity)) =
                control_points_parent_query.get_mut(*edited_level_object_entity)
            {
                let mut points = Vec::new();
                for point in &edited_level_object.desc.control_points() {
                    let mut entity_commands = commands.spawn();
                    entity_commands
                        .insert_bundle(PbrBundle {
                            mesh: muddle_assets.meshes.control_point.clone(),
                            material: muddle_assets.materials.control_point_normal.clone(),
                            transform: Transform::from_translation(point.extend(0.0)),
                            ..Default::default()
                        })
                        .insert(Parent(*ghost_entity))
                        .insert(LevelObjectControlPoint)
                        .insert_bundle(bevy_mod_picking::PickableBundle::default());
                    points.push(entity_commands.id());
                }
                if !points.is_empty() {
                    commands
                        .entity(*edited_level_object_entity)
                        .insert(Children::with(&points))
                        .insert(LevelObjectControlPoints { points });
                }
            }
        }
    }

    if prev_edited_level_object_entity.is_some() {
        for point in control_points_query.iter() {
            commands.entity(point).despawn();
        }
    }
}

#[derive(Default)]
pub struct ControlPointsState {
    is_being_dragged: bool,
    start_position: Option<Vec2>,
    prev_hovered_point: Option<Entity>,
    prev_edited_level_object: Option<Entity>,
}

#[derive(SystemParam)]
pub struct ControlPointsQueries<'a> {
    control_point_parent_query: Query<
        'a,
        (
            &'static LevelObjectStaticGhostParent,
            &'static LevelObjectControlPoints,
        ),
    >,
    control_points_query: Query<
        'a,
        (
            &'static mut Transform,
            &'static mut Handle<StandardMaterial>,
        ),
        With<LevelObjectControlPoint>,
    >,
    control_point_parent_ghost_query: Query<
        'a,
        &'static Transform,
        (
            With<LevelObjectStaticGhost>,
            Without<LevelObjectControlPoint>,
        ),
    >,
}

pub fn process_control_points_input(
    mouse_input: MouseInput,
    mut edited_level_object: ResMut<EditedLevelObject>,
    muddle_assets: MuddleAssets,
    mut level_object_requests: ResMut<LevelObjectRequestsQueue>,
    mut control_points_queries: ControlPointsQueries,
    // Screen coordinates at where the dragging started.
    mut control_points_state: Local<ControlPointsState>,
) {
    let EditedLevelObject {
        object,
        dragged_control_point_index: dragged_control_point_index_state,
        ..
    } = &mut *edited_level_object;
    let (edited_object, level_object) = match object.as_mut() {
        Some(level_object) => level_object,
        None => return,
    };

    if Some(*edited_object) != control_points_state.prev_edited_level_object {
        control_points_state.prev_edited_level_object = Some(*edited_object);
        control_points_state.is_being_dragged = false;
        control_points_state.start_position = None;
        control_points_state.prev_hovered_point = None;
        *dragged_control_point_index_state = None;
        // Newly spawned control points will be available next run.
        return;
    }

    if control_points_state.start_position.is_none() {
        let new_hovered_entity = mouse_input.mouse_entity_picker.hovered_entity();
        if new_hovered_entity != control_points_state.prev_hovered_point {
            if let Some(prev_hovered_point) = control_points_state.prev_hovered_point {
                if let Ok(mut point_material) = control_points_queries
                    .control_points_query
                    .get_component_mut::<Handle<StandardMaterial>>(prev_hovered_point)
                {
                    *point_material = muddle_assets.materials.control_point_normal.clone();
                }
            }
            control_points_state.prev_hovered_point = None;
            if let Some(new_hovered_entity) = new_hovered_entity {
                if let Ok(mut point_material) = control_points_queries
                    .control_points_query
                    .get_component_mut::<Handle<StandardMaterial>>(new_hovered_entity)
                {
                    *point_material = muddle_assets.materials.control_point_hovered.clone();
                    control_points_state.prev_hovered_point = Some(new_hovered_entity);
                }
            }
        }
    }

    if mouse_input
        .mouse_button_input
        .just_pressed(MouseButton::Left)
    {
        if let Some(hovered_point) = control_points_state.prev_hovered_point {
            let LevelObjectControlPoints { points } = control_points_queries
                .control_point_parent_query
                .get_component::<LevelObjectControlPoints>(*edited_object)
                .unwrap();
            if let Some(index) = points.iter().position(|point| *point == hovered_point) {
                *dragged_control_point_index_state = Some(index);
                control_points_state.start_position = Some(mouse_input.mouse_screen_position.0);
            }
        }
    }

    let dragged_control_point_index = match dragged_control_point_index_state {
        Some(dragged_control_point_index) => dragged_control_point_index,
        None => return,
    };

    let dragging_threshold_squared = 100.0;
    if control_points_state
        .start_position
        .unwrap()
        .distance_squared(mouse_input.mouse_screen_position.0)
        > dragging_threshold_squared
    {
        control_points_state.is_being_dragged = true;
    }

    if mouse_input.mouse_button_input.pressed(MouseButton::Left) {
        if control_points_state.is_being_dragged {
            let LevelObjectStaticGhostParent(ghost_entity) = control_points_queries
                .control_point_parent_query
                .get_component::<LevelObjectStaticGhostParent>(*edited_object)
                .unwrap();
            let ghost_transform = control_points_queries
                .control_point_parent_ghost_query
                .get(*ghost_entity)
                .unwrap();
            let new_translation =
                mouse_input.mouse_world_position.0.extend(0.0) - ghost_transform.translation;
            if let Ok(mut point_transform) = control_points_queries
                .control_points_query
                .get_component_mut::<Transform>(control_points_state.prev_hovered_point.unwrap())
            {
                point_transform.translation = new_translation;
            }
            level_object
                .desc
                .set_control_point(*dragged_control_point_index, new_translation.xy());
        }
    } else {
        if control_points_state.is_being_dragged {
            level_object_requests
                .update_requests
                .push(level_object.clone());
        }
        *dragged_control_point_index_state = None;
        control_points_state.start_position = None;
        control_points_state.is_being_dragged = false;
    }
}
