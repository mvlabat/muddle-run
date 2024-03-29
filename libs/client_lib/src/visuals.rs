use crate::{
    components::{
        LevelObjectControlBorder, LevelObjectControlBorders, LevelObjectControlPoint,
        LevelObjectControlPoints,
    },
    helpers::PlayerParams,
    input::LevelObjectRequestsQueue,
    ui::builder_ui::{EditedLevelObject, MouseInput},
};
use bevy::{
    asset::{Assets, Handle},
    ecs::{
        entity::Entity,
        query::{Or, With, Without},
        system::{Commands, Local, Query, Res, ResMut, SystemParam},
    },
    hierarchy::BuildChildren,
    input::mouse::MouseButton,
    math::{Quat, Vec2, Vec3, Vec3Swizzles},
    pbr::{PbrBundle, StandardMaterial},
    render::{mesh::Mesh, view::Visibility},
    transform::components::Transform,
};
use mr_shared_lib::{
    client::{
        assets::{MuddleAssets, MuddleMaterials},
        XyPlane,
    },
    game::{
        client_factories::VisibilitySettings,
        components::{
            LevelObjectStaticGhostChild, LevelObjectStaticGhostParent, LevelObjectTag,
            PlayerSensor, PlayerSensors, Spawned,
        },
        level::{CollisionLogic, LevelObjectDesc, LevelParams},
    },
    player::PlayerRole,
    GameTime,
};

pub fn control_builder_visibility_system(
    mut prev_role: Local<Option<PlayerRole>>,
    player_params: PlayerParams,
    level_params: LevelParams,
    mut visibility_settings: ResMut<VisibilitySettings>,
    mut level_objects_query: Query<(Entity, &Transform, &mut Visibility), With<LevelObjectTag>>,
    mut ghosts_query: Query<
        (&Transform, &mut Visibility, &LevelObjectStaticGhostParent),
        Without<LevelObjectTag>,
    >,
) {
    #[cfg(feature = "profiler")]
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

        for (transform, mut visible, LevelObjectStaticGhostParent(parent_entity)) in
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

pub type ControlEntitiesQuery<'w, 's> = Query<
    'w,
    's,
    Entity,
    Or<(
        With<LevelObjectControlPoint>,
        With<LevelObjectControlBorder>,
    )>,
>;

pub fn spawn_control_points_system(
    mut commands: Commands,
    muddle_assets: MuddleAssets,
    mut meshes: ResMut<Assets<Mesh>>,
    edited_level_object: Res<EditedLevelObject>,
    mut prev_edited_level_object: Local<Option<Entity>>,
    mut control_points_parent_query: Query<&LevelObjectStaticGhostChild>,
    control_entities_query: ControlEntitiesQuery,
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
            if let Ok(LevelObjectStaticGhostChild(ghost_entity)) =
                control_points_parent_query.get_mut(*edited_level_object_entity)
            {
                let mut points = Vec::new();
                let control_points = edited_level_object.desc.control_points();
                for point in &control_points {
                    commands.entity(*ghost_entity).with_children(|parent| {
                        let mut entity_commands = parent.spawn_empty();
                        entity_commands
                            .insert(PbrBundle {
                                mesh: muddle_assets.meshes.control_point.clone(),
                                material: muddle_assets.materials.control_point_normal.clone(),
                                transform: Transform::from_translation(point.extend(0.0)),
                                ..Default::default()
                            })
                            .insert(LevelObjectControlPoint)
                            .insert(bevy_mod_picking::PickableBundle::default());
                        points.push(entity_commands.id());
                    });
                }
                let lines = (0..control_points.len())
                    .filter_map(|i| {
                        if i == 1 && control_points.len() == 2 {
                            return None;
                        }

                        let border_line =
                            control_points[(i + 1) % control_points.len()] - control_points[i];
                        let length = border_line.length();
                        if length < f32::EPSILON {
                            return None;
                        }

                        let mut entity_commands = commands.spawn_empty();
                        entity_commands
                            .insert(PbrBundle {
                                mesh: meshes.add(Mesh::from(XyPlane {
                                    size: Vec2::new(length, 0.04),
                                })),
                                material: muddle_assets.materials.control_point_normal.clone(),
                                transform: Transform {
                                    translation: (control_points[i] + border_line / 2.0)
                                        .extend(0.01),
                                    rotation: Quat::from_rotation_arc(
                                        Vec3::X,
                                        border_line.normalize().extend(0.0),
                                    ),
                                    scale: Vec3::ONE,
                                },
                                ..Default::default()
                            })
                            .insert(LevelObjectControlBorder)
                            .insert(bevy_mod_picking::PickableBundle::default());
                        let spawned_entity = entity_commands.id();
                        entity_commands
                            .commands()
                            .entity(*ghost_entity)
                            .add_child(spawned_entity);
                        Some((i, spawned_entity))
                    })
                    .collect::<Vec<_>>();
                if !points.is_empty() {
                    commands
                        .entity(*edited_level_object_entity)
                        .push_children(&points)
                        .insert(LevelObjectControlPoints { points })
                        .insert(LevelObjectControlBorders { lines });
                }
            }
        }
    }

    if prev_edited_level_object_entity.is_some() {
        for point in control_entities_query.iter() {
            commands.entity(point).despawn();
        }
    }
}

pub type ControlEntitiesQueryMutComponents = (
    &'static mut Transform,
    &'static mut Handle<StandardMaterial>,
    &'static mut Handle<Mesh>,
);

pub type ControlEntitiesQueryMutFilter = Or<(
    With<LevelObjectControlPoint>,
    With<LevelObjectControlBorder>,
)>;

pub type ControlPointParentQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static LevelObjectStaticGhostChild,
        &'static LevelObjectControlPoints,
        &'static LevelObjectControlBorders,
    ),
>;
pub type ControlEntitiesQueryMut<'w, 's> =
    Query<'w, 's, ControlEntitiesQueryMutComponents, ControlEntitiesQueryMutFilter>;
pub type ControlPointParentGhostQuery<'w, 's> = Query<
    'w,
    's,
    &'static Transform,
    (
        With<LevelObjectStaticGhostParent>,
        Without<LevelObjectControlPoint>,
        Without<LevelObjectControlBorder>,
    ),
>;

#[derive(SystemParam)]
pub struct ControlPointsQueries<'w, 's> {
    control_point_parent_query: ControlPointParentQuery<'w, 's>,
    control_entities_query: ControlEntitiesQueryMut<'w, 's>,
    control_point_parent_ghost_query: ControlPointParentGhostQuery<'w, 's>,
}

pub fn process_control_points_input_system(
    mut mouse_input: MouseInput<ControlEntitiesQueryMutComponents, ControlEntitiesQueryMutFilter>,
    mut edited_level_object: ResMut<EditedLevelObject>,
    muddle_assets: MuddleAssets,
    mut meshes: ResMut<Assets<Mesh>>,
    mut level_object_requests: ResMut<LevelObjectRequestsQueue>,
    mut control_points_queries: ControlPointsQueries,
    // Screen coordinates at where the dragging started.
    mut prev_edited_level_object: Local<Option<Entity>>,
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

    if Some(*edited_object) != *prev_edited_level_object {
        *prev_edited_level_object = Some(*edited_object);
        *dragged_control_point_index_state = None;
        mouse_input.mouse_entity_picker.reset();
        // Newly spawned control points will be available next run.
        return;
    }

    mouse_input.mouse_entity_picker.process_input(&mut Some(
        &mut control_points_queries.control_entities_query,
    ));

    let prev_hovered_entity = mouse_input
        .mouse_entity_picker
        .prev_state()
        .picked_entity
        .or_else(|| mouse_input.mouse_entity_picker.prev_state().hovered_entity);
    let hovered_entity = mouse_input
        .mouse_entity_picker
        .state()
        .picked_entity
        .or_else(|| mouse_input.mouse_entity_picker.state().hovered_entity);
    if prev_hovered_entity != hovered_entity {
        if let Some(prev_hovered_point) = prev_hovered_entity {
            if let Ok(mut point_material) = control_points_queries
                .control_entities_query
                .get_component_mut::<Handle<StandardMaterial>>(prev_hovered_point)
            {
                *point_material = muddle_assets.materials.control_point_normal.clone();
            }
        }
        if let Some(new_hovered_entity) = hovered_entity {
            if let Ok(mut point_material) = control_points_queries
                .control_entities_query
                .get_component_mut::<Handle<StandardMaterial>>(new_hovered_entity)
            {
                *point_material = muddle_assets.materials.control_point_hovered.clone();
            }
        }
    }

    if mouse_input
        .mouse_button_input
        .just_pressed(MouseButton::Left)
    {
        if let Some(hovered_point) = prev_hovered_entity {
            let LevelObjectControlPoints { points } = control_points_queries
                .control_point_parent_query
                .get_component::<LevelObjectControlPoints>(*edited_object)
                .unwrap();
            if let Some(index) = points.iter().position(|point| *point == hovered_point) {
                *dragged_control_point_index_state = Some(index);
            }
        }
    }

    if !mouse_input.mouse_entity_picker.state().is_dragged
        && mouse_input.mouse_entity_picker.prev_state().is_dragged
    {
        level_object_requests
            .update_requests
            .push(level_object.clone());
        *dragged_control_point_index_state = None;
    }

    let dragged_control_point_index = match dragged_control_point_index_state {
        Some(dragged_control_point_index) => dragged_control_point_index,
        None => return,
    };

    let LevelObjectStaticGhostChild(ghost_entity) = control_points_queries
        .control_point_parent_query
        .get_component::<LevelObjectStaticGhostChild>(*edited_object)
        .unwrap();
    let ghost_transform = control_points_queries
        .control_point_parent_ghost_query
        .get(*ghost_entity)
        .unwrap();
    let new_translation =
        mouse_input.mouse_world_position.0.extend(0.0) - ghost_transform.translation;
    if let Ok(mut point_transform) = control_points_queries
        .control_entities_query
        .get_component_mut::<Transform>(hovered_entity.unwrap())
    {
        point_transform.translation = new_translation;
    }
    let new_point_pos = new_translation.xy();
    level_object
        .desc
        .set_control_point(*dragged_control_point_index, new_translation.xy());

    let LevelObjectControlPoints { points } = control_points_queries
        .control_point_parent_query
        .get_component::<LevelObjectControlPoints>(*edited_object)
        .unwrap();
    let connected_to = points
        .iter()
        .enumerate()
        .find(|(i, _)| *i == (*dragged_control_point_index + 1) % points.len());
    let connected_from = points.iter().enumerate().find(|(i, _)| {
        if *dragged_control_point_index == 0 {
            *i == points.len() - 1
        } else {
            *i == *dragged_control_point_index - 1
        }
    });
    let LevelObjectControlBorders { lines } = control_points_queries
        .control_point_parent_query
        .get_component::<LevelObjectControlBorders>(*edited_object)
        .unwrap();

    // TODO: dedup.
    if let Some((point_index, connected_to)) = connected_to {
        let line_index = if point_index == 0 {
            lines.len() - 1
        } else {
            point_index - 1
        };
        let connected_to_pos = control_points_queries
            .control_entities_query
            .get_component::<Transform>(*connected_to)
            .unwrap()
            .translation
            .xy();
        let border_line = connected_to_pos - new_point_pos;
        let length = border_line.length();
        if length > f32::EPSILON {
            let mut transform = control_points_queries
                .control_entities_query
                .get_component_mut::<Transform>(lines[line_index].1)
                .unwrap();
            transform.translation = (new_point_pos + border_line / 2.0).extend(0.01);
            transform.rotation =
                Quat::from_rotation_arc(Vec3::X, border_line.normalize().extend(0.0));
            let mut mesh = control_points_queries
                .control_entities_query
                .get_component_mut::<Handle<Mesh>>(lines[line_index].1)
                .unwrap();
            meshes.remove(mesh.clone_weak());
            *mesh = meshes.add(Mesh::from(XyPlane {
                size: Vec2::new(length, 0.04),
            }));
        }
    }
    if let Some((point_index, connected_from)) = connected_from {
        let line_index = point_index;
        let connected_from_pos = control_points_queries
            .control_entities_query
            .get_component::<Transform>(*connected_from)
            .unwrap()
            .translation
            .xy();
        let border_line = new_point_pos - connected_from_pos;
        let length = border_line.length();
        if length > f32::EPSILON {
            let mut transform = control_points_queries
                .control_entities_query
                .get_component_mut::<Transform>(lines[line_index].1)
                .unwrap();
            transform.translation = (connected_from_pos + border_line / 2.0).extend(0.01);
            transform.rotation =
                Quat::from_rotation_arc(Vec3::X, border_line.normalize().extend(0.0));
            let mut mesh = control_points_queries
                .control_entities_query
                .get_component_mut::<Handle<Mesh>>(lines[line_index].1)
                .unwrap();
            meshes.remove(mesh.clone_weak());
            *mesh = meshes.add(Mesh::from(XyPlane {
                size: Vec2::new(length, 0.04),
            }));
        }
    }
}

pub fn update_player_sensor_materials_system(
    time: Res<GameTime>,
    players: Query<(&PlayerSensors, &Spawned)>,
    mut player_sensor_materials: Query<&mut Handle<StandardMaterial>, With<PlayerSensor>>,
    visibility_settings: Res<VisibilitySettings>,
    muddle_materials: Res<MuddleMaterials>,
) {
    if !visibility_settings.debug {
        return;
    }

    for (player_sensors, spawned) in players.iter() {
        if !spawned.is_spawned(time.frame_number) {
            continue;
        }

        for (sensor_entity, sensor_state) in &player_sensors.sensors {
            let mut material = player_sensor_materials.get_mut(*sensor_entity).unwrap();
            if sensor_state.contacting.is_empty() || sensor_state.has(CollisionLogic::Death) {
                *material = muddle_materials.player_sensor_death.clone();
            } else {
                *material = muddle_materials.player_sensor_normal.clone();
            }
        }
    }
}
