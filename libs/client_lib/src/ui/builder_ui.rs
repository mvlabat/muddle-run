use crate::{
    helpers::MouseEntityPicker, input::LevelObjectRequestsQueue, CurrentPlayerNetId,
    LevelObjectCorrelations,
};
use bevy::{
    ecs::{
        entity::Entity,
        system::{Local, Query, Res, ResMut, SystemParam},
    },
    log,
    math::Vec2,
};
use bevy_egui::{egui, EguiContext};
use mr_shared_lib::{
    game::{
        components::LevelObjectLabel,
        level::{LevelObject, LevelObjectDesc, LevelState},
        level_objects::{CubeDesc, PlaneDesc},
    },
    messages::{EntityNetId, PlayerNetId, SpawnLevelObjectRequest, SpawnLevelObjectRequestBody},
    net::MessageId,
    player::{Player, PlayerRole},
    registry::EntityRegistry,
};
use std::collections::HashMap;

pub struct PickedLevelObject {
    entity: Entity,
    entity_net_id: EntityNetId,
    desc: LevelObjectDesc,
    dirty_desc: LevelObjectDesc,
}

#[derive(SystemParam)]
pub struct LevelObjects<'a> {
    pending_correlation: Local<'a, Option<MessageId>>,
    picked_level_object: Local<'a, Option<PickedLevelObject>>,
    level_state: Res<'a, LevelState>,
    entity_registry: Res<'a, EntityRegistry<EntityNetId>>,
    level_object_query: Query<'a, &'static LevelObjectLabel>,
}

pub fn builder_ui(
    egui_context: ResMut<EguiContext>,
    mut mouse_entity_picker: MouseEntityPicker,
    current_player_net_id: Res<CurrentPlayerNetId>,
    players: Res<HashMap<PlayerNetId, Player>>,
    mut level_object_correlations: ResMut<LevelObjectCorrelations>,
    mut level_object_requests: ResMut<LevelObjectRequestsQueue>,
    mut level_objects: LevelObjects,
) {
    let current_player_id = match current_player_net_id.0 {
        Some(current_player_id) => current_player_id,
        None => {
            *level_objects.picked_level_object = None;
            return;
        }
    };
    let player = players
        .get(&current_player_id)
        .expect("Expected a current player to exist");
    if !matches!(player.role, PlayerRole::Builder) {
        *level_objects.picked_level_object = None;
        return;
    }

    // Picking a level object if we received a confirmation from the server about an object created
    // by us.
    if let Some(correlation_id) = *level_objects.pending_correlation {
        if let Some(entity_net_id) = level_object_correlations.query(correlation_id) {
            *level_objects.picked_level_object = level_objects
                .entity_registry
                .get_entity(entity_net_id)
                .zip(
                    level_objects
                        .level_state
                        .objects
                        .get(&entity_net_id)
                        .cloned(),
                )
                .map(|(entity, level_object)| PickedLevelObject {
                    entity,
                    entity_net_id,
                    desc: level_object.desc.clone(),
                    dirty_desc: level_object.desc,
                });
            if level_objects.picked_level_object.is_none() {
                log::error!("Level object {} isn't registered", entity_net_id.0);
            }
            *level_objects.pending_correlation = None;
        }
    }

    // Picking a level object with a mouse.
    if !egui_context.ctx().is_pointer_over_area() {
        mouse_entity_picker.pick_entity();
    }
    if let Some((entity, entity_net_id, picked_level_object)) = mouse_entity_picker
        .take_picked_entity()
        .and_then(|entity| {
            level_objects
                .entity_registry
                .get_id(entity)
                .map(|entity_net_id| (entity, entity_net_id))
        })
        .and_then(|(entity, entity_net_id)| {
            level_objects
                .level_state
                .objects
                .get(&entity_net_id)
                .map(|level_object| (entity, entity_net_id, level_object.clone()))
        })
    {
        // We don't reset edited state if the clicked object is the same.
        if !matches!(*level_objects.picked_level_object, Some(PickedLevelObject { entity: picked_entity, .. }) if picked_entity == entity)
        {
            *level_objects.picked_level_object = Some(PickedLevelObject {
                entity,
                entity_net_id,
                desc: picked_level_object.desc.clone(),
                dirty_desc: picked_level_object.desc,
            });
            *level_objects.pending_correlation = None;
        }
    }

    if level_objects.picked_level_object.is_some() {
        // When an object is updated, it may get re-spawned as a new entity. We need to update
        // the picked entity in such a case. Despawns may happen as well.
        if let Some(level_object_entity) = level_objects.entity_registry.get_entity(
            level_objects
                .picked_level_object
                .as_ref()
                .unwrap()
                .entity_net_id,
        ) {
            level_objects.picked_level_object.as_mut().unwrap().entity = level_object_entity;
        } else {
            *level_objects.picked_level_object = None;
        }
    }

    let ctx = egui_context.ctx();
    egui::Window::new("Builder menu").show(ctx, |ui| {
        if let Some(PickedLevelObject {
            entity,
            entity_net_id,
            dirty_desc,
            desc,
        }) = &mut *level_objects.picked_level_object
        {
            let level_object_label = level_objects
                .level_object_query
                .get(*entity)
                .expect("Expected a label for a created level object");

            egui::Grid::new("editing_picked_level_object")
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Editing");
                    ui.label(&level_object_label.0);
                    ui.end_row();

                    ui.label("Position");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::widgets::DragValue::new(&mut dirty_desc.position_mut().x)
                                .speed(0.1),
                        );
                        ui.add(
                            egui::widgets::DragValue::new(&mut dirty_desc.position_mut().y)
                                .speed(0.1),
                        );
                    });
                    ui.end_row();

                    match dirty_desc {
                        LevelObjectDesc::Cube(CubeDesc { size, .. })
                        | LevelObjectDesc::Plane(PlaneDesc { size, .. }) => {
                            ui.label("Size");
                            ui.add(egui::widgets::DragValue::new(size).speed(0.01));
                            ui.end_row();
                        }
                    }

                    ui.label("Actions");
                    ui.horizontal(|ui| {
                        if ui.button("Despawn").clicked() {
                            level_object_requests.despawn_requests.push(*entity_net_id);
                        }
                    });
                    ui.end_row();
                });

            if dirty_desc != desc {
                level_object_requests.update_requests.push(LevelObject {
                    net_id: *entity_net_id,
                    desc: dirty_desc.clone(),
                });
                *desc = dirty_desc.clone();
            }
            ui.separator();
        }

        ui.label("Create new object:");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Plane").clicked() {
                let correlation_id = level_object_correlations.next_correlation_id();
                *level_objects.pending_correlation = Some(correlation_id);
                level_object_requests
                    .spawn_requests
                    .push(SpawnLevelObjectRequest {
                        correlation_id,
                        body: SpawnLevelObjectRequestBody::New(LevelObjectDesc::Plane(PlaneDesc {
                            position: Vec2::new(0.0, 0.0),
                            size: 50.0,
                        })),
                    });
            }
            if ui.button("Cube").clicked() {
                let correlation_id = level_object_correlations.next_correlation_id();
                *level_objects.pending_correlation = Some(correlation_id);
                level_object_requests
                    .spawn_requests
                    .push(SpawnLevelObjectRequest {
                        correlation_id,
                        body: SpawnLevelObjectRequestBody::New(LevelObjectDesc::Cube(CubeDesc {
                            position: Vec2::new(5.0, 5.0),
                            size: 0.4,
                        })),
                    });
            }
        });
    });
}
