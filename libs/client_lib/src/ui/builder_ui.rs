use crate::{
    helpers::{MouseEntityPicker, PlayerParams},
    input::LevelObjectRequestsQueue,
    ui::widgets::sortable::{sortable_list, ListItem},
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
use bevy_egui::{egui, egui::Ui, EguiContext};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        components::{LevelObjectLabel, LevelObjectStaticGhost, Spawned},
        level::{LevelObject, LevelObjectDesc, LevelState, ObjectRoute, ObjectRouteDesc},
        level_objects::{CubeDesc, PlaneDesc, RoutePointDesc},
    },
    messages::{EntityNetId, SpawnLevelObjectRequest, SpawnLevelObjectRequestBody},
    net::MessageId,
    player::PlayerRole,
    registry::EntityRegistry,
    GameTime, SIMULATIONS_PER_SECOND,
};
use std::collections::HashMap;

pub const DEFAULT_PERIOD: FrameNumber = FrameNumber::new(SIMULATIONS_PER_SECOND * 10);

#[derive(Clone)]
pub struct PickedLevelObject {
    entity: Entity,
    level_object: LevelObject,
    dirty_level_object: LevelObject,
}

#[derive(SystemParam)]
pub struct LevelObjects<'a> {
    time: Res<'a, GameTime>,
    pending_correlation: Local<'a, Option<MessageId>>,
    picked_level_object: Local<'a, Option<PickedLevelObject>>,
    level_state: Res<'a, LevelState>,
    entity_registry: Res<'a, EntityRegistry<EntityNetId>>,
    query: Query<'a, (Entity, &'static LevelObjectLabel, &'static Spawned)>,
    ghosts_query: Query<'a, &'static LevelObjectStaticGhost>,
}

#[derive(Default)]
pub struct BuilderUiState {
    filter: String,
}

pub fn builder_ui(
    egui_context: ResMut<EguiContext>,
    mut builder_ui_state: Local<BuilderUiState>,
    mut mouse_entity_picker: MouseEntityPicker,
    player_params: PlayerParams,
    mut level_object_correlations: ResMut<LevelObjectCorrelations>,
    mut level_object_requests: ResMut<LevelObjectRequestsQueue>,
    mut level_objects: LevelObjects,
) {
    let player = match player_params.current_player() {
        Some(player) => player,
        None => {
            *level_objects.picked_level_object = None;
            return;
        }
    };
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
                    dirty_level_object: level_object.clone(),
                    level_object,
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
    if let Some((entity, _, picked_level_object)) = mouse_entity_picker
        .take_picked_entity()
        .and_then(|entity| {
            // Checking whether we've clicked a ghost.
            if let Ok(LevelObjectStaticGhost(ghost_parent)) = level_objects.ghosts_query.get(entity)
            {
                return Some((
                    *ghost_parent,
                    level_objects.entity_registry.get_id(*ghost_parent).unwrap(),
                ));
            }
            // Checking normal objects.
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
                level_object: picked_level_object.clone(),
                dirty_level_object: picked_level_object,
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
                .level_object
                .net_id,
        ) {
            if level_objects
                .query
                .get_component::<Spawned>(level_object_entity)
                .unwrap()
                .is_spawned(level_objects.time.frame_number)
            {
                level_objects.picked_level_object.as_mut().unwrap().entity = level_object_entity;
            } else {
                *level_objects.picked_level_object = None;
            }
        } else {
            *level_objects.picked_level_object = None;
        }
    }

    let ctx = egui_context.ctx();
    egui::Window::new("Builder menu").show(ctx, |ui| {
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
            if ui.button("Route point").clicked() {
                let correlation_id = level_object_correlations.next_correlation_id();
                *level_objects.pending_correlation = Some(correlation_id);
                level_object_requests
                    .spawn_requests
                    .push(SpawnLevelObjectRequest {
                        correlation_id,
                        body: SpawnLevelObjectRequestBody::New(LevelObjectDesc::RoutePoint(
                            RoutePointDesc {
                                position: Vec2::new(-5.0, 5.0),
                            },
                        )),
                    });
            }
        });

        if let Some(PickedLevelObject {
            entity: _,
            level_object,
            mut dirty_level_object,
        }) = level_objects.picked_level_object.clone()
        {
            level_object_ui(
                &mut level_object_requests,
                ui,
                &level_object,
                &mut dirty_level_object,
            );

            if dirty_level_object.desc.position().is_some() {
                route_settings(
                    ui,
                    &mut builder_ui_state,
                    &mut level_objects,
                    &mut dirty_level_object,
                );
            }

            if level_object != dirty_level_object {
                assert_eq!(level_object.net_id, dirty_level_object.net_id);
                level_object_requests.update_requests.push(LevelObject {
                    net_id: level_object.net_id,
                    label: dirty_level_object.label.clone(),
                    desc: dirty_level_object.desc.clone(),
                    route: dirty_level_object.route.clone(),
                });

                let picked_level_object = level_objects.picked_level_object.as_mut().unwrap();
                picked_level_object.level_object = dirty_level_object.clone();
                picked_level_object.dirty_level_object = dirty_level_object;
            }
        }
    });
}

fn level_object_ui(
    level_object_requests: &mut LevelObjectRequestsQueue,
    ui: &mut Ui,
    level_object: &LevelObject,
    mut dirty_level_object: &mut LevelObject,
) {
    ui.separator();
    egui::Grid::new("editing_picked_level_object")
        .striped(true)
        .show(ui, |ui| {
            ui.label("Object label");
            ui.text_edit_singleline(&mut dirty_level_object.label);
            ui.end_row();

            if let Some(pos) = dirty_level_object.desc.position_mut() {
                ui.label("Position");
                ui.horizontal(|ui| {
                    ui.add(egui::widgets::DragValue::new(&mut pos.x).speed(0.1));
                    ui.add(egui::widgets::DragValue::new(&mut pos.y).speed(0.1));
                });
                ui.end_row();
            }

            match &mut dirty_level_object.desc {
                LevelObjectDesc::Cube(CubeDesc { size, .. })
                | LevelObjectDesc::Plane(PlaneDesc { size, .. }) => {
                    ui.label("Size");
                    ui.add(egui::widgets::DragValue::new(size).speed(0.01));
                    ui.end_row();
                }
                LevelObjectDesc::RoutePoint(_) => {}
            }

            ui.label("Actions");
            ui.horizontal(|ui| {
                if ui.button("Despawn").clicked() {
                    level_object_requests
                        .despawn_requests
                        .push(level_object.net_id);
                }
            });
            ui.end_row();

            if dirty_level_object.desc.position().is_some() {
                ui.label("Route type");
                route_type(ui, &mut dirty_level_object);
                ui.end_row();

                if let Some(route) = &mut dirty_level_object.route {
                    // We want to hide period and start offset settings for the Attached
                    // route type.
                    if !matches!(route.desc, ObjectRouteDesc::Attached(_)) {
                        // Period may be equal 0 if we are switching from the Attached route
                        // type to another one.
                        if route.period == FrameNumber::new(0) {
                            route.period =
                                DEFAULT_PERIOD.min(route.start_frame_offset + FrameNumber::new(1));
                        }

                        ui.label("Period (frames)");
                        ui.add(
                            egui::widgets::DragValue::new(&mut route.period)
                                .speed(0.1)
                                .clamp_range(
                                    SIMULATIONS_PER_SECOND.max(route.start_frame_offset.value() + 1)
                                        ..=SIMULATIONS_PER_SECOND * 60,
                                ),
                        );
                        ui.end_row();

                        ui.label("Period (second)");
                        ui.label(format!(
                            "{:.2}",
                            route.period.value() as f32 / SIMULATIONS_PER_SECOND as f32
                        ));
                        ui.end_row();

                        ui.label("Start offset (frames)");
                        ui.add(
                            egui::widgets::DragValue::new(&mut route.start_frame_offset)
                                .speed(0.1)
                                .clamp_range(
                                    FrameNumber::new(0)..=route.period - FrameNumber::new(1),
                                ),
                        );
                    } else {
                        // Attached and Radial route types actually behave the same, we
                        // just display this difference in the UI and set these values
                        // to 0 for the Attached type under the hood to prevent objects
                        // from making circles.
                        route.period = FrameNumber::new(0);
                        route.start_frame_offset = FrameNumber::new(0);
                    }
                }
            }
        });
}

fn route_settings(
    ui: &mut egui::Ui,
    builder_ui_state: &mut BuilderUiState,
    level_objects: &mut LevelObjects,
    dirty_level_object: &mut LevelObject,
) {
    let dirty_level_object_route = match &mut dirty_level_object.route {
        Some(route) => route,
        None => return,
    };

    let response = egui::CollapsingHeader::new("Route settings").show(ui, |ui| {
        match &mut dirty_level_object_route.desc {
            ObjectRouteDesc::Attached(route_point) | ObjectRouteDesc::Radial(route_point) => {
                let point_label = route_point
                    .and_then(|point| level_objects.level_state.objects.get(&point))
                    .map_or("None".to_owned(), |level_object| level_object.label.clone());
                ui.label(format!("Route point: {}", point_label));
            }
            ObjectRouteDesc::ForwardCycle(route_points)
            | ObjectRouteDesc::ForwardBackwardsCycle(route_points) => {
                let mut list = Vec::new();
                let mut duplicate_counts = HashMap::new();
                for point in &*route_points {
                    if let Some(level_object) = level_objects.level_state.objects.get(point) {
                        let n = duplicate_counts
                            .entry(Some(*point))
                            .and_modify(|count| *count += 1)
                            .or_insert(0);
                        list.push(ListItem {
                            id: egui::Id::new(level_object.net_id).with(n),
                            label: level_object.label.clone(),
                            data: *point,
                            sortable: true,
                        });
                    } else {
                        let n = duplicate_counts
                            .entry(None)
                            .and_modify(|count| *count += 1)
                            .or_insert(0);
                        list.push(ListItem {
                            id: egui::Id::new("invalid").with(n),
                            label: "<Invalid>".to_owned(),
                            data: *point,
                            sortable: true,
                        });
                    }
                }

                let edited = sortable_list(ui, "route settings", &mut list);
                if edited {
                    *route_points = list.into_iter().map(|list_item| list_item.data).collect();
                }
            }
        }
    });

    if response.body_returned.is_some() {
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut builder_ui_state.filter);
            if ui.button("❌").clicked() {
                builder_ui_state.filter = String::new();
            }
        });
        egui::ScrollArea::auto_sized().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (entity, label, spawned) in level_objects.query.iter() {
                    if !spawned.is_spawned(level_objects.time.frame_number) {
                        continue;
                    }

                    if !label
                        .0
                        .to_lowercase()
                        .contains(&builder_ui_state.filter.to_lowercase())
                    {
                        continue;
                    }

                    if ui.button(&label.0).clicked() {
                        let selected_entity_net_id = level_objects
                            .entity_registry
                            .get_id(entity)
                            .expect("Expected a registered level object");
                        match &mut dirty_level_object_route.desc {
                            ObjectRouteDesc::Attached(route_point)
                            | ObjectRouteDesc::Radial(route_point) => {
                                *route_point = Some(selected_entity_net_id);
                            }
                            ObjectRouteDesc::ForwardCycle(route_points)
                            | ObjectRouteDesc::ForwardBackwardsCycle(route_points) => {
                                route_points.push(selected_entity_net_id);
                            }
                        }
                    }
                }
            });
        });
    }
}

fn route_type(ui: &mut egui::Ui, dirty_level_object: &mut LevelObject) {
    #[derive(Copy, Clone, PartialEq, Debug)]
    enum Type {
        Stationary,
        Attached,
        Radial,
        ForwardCycle,
        ForwardBackwardsCycle,
    }

    impl std::fmt::Display for Type {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Type::Stationary => write!(f, "Stationary"),
                Type::Attached => write!(f, "Attached"),
                Type::Radial => write!(f, "Radial"),
                Type::ForwardCycle => write!(f, "Forward Cycle"),
                Type::ForwardBackwardsCycle => write!(f, "Forward Backwards Cycle"),
            }
        }
    }

    let route_type = match dirty_level_object.route {
        None => Type::Stationary,
        Some(ObjectRoute {
            desc: ObjectRouteDesc::Attached(_),
            ..
        }) => Type::Attached,
        Some(ObjectRoute {
            desc: ObjectRouteDesc::Radial(_),
            ..
        }) => Type::Radial,
        Some(ObjectRoute {
            desc: ObjectRouteDesc::ForwardCycle(_),
            ..
        }) => Type::ForwardCycle,
        Some(ObjectRoute {
            desc: ObjectRouteDesc::ForwardBackwardsCycle(_),
            ..
        }) => Type::ForwardBackwardsCycle,
    };
    let mut dirty_route_type = route_type;

    egui::containers::ComboBox::from_label("")
        .width(200.0)
        .selected_text(route_type.to_string())
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut dirty_route_type,
                Type::Stationary,
                Type::Stationary.to_string(),
            );
            ui.selectable_value(
                &mut dirty_route_type,
                Type::Attached,
                Type::Attached.to_string(),
            );
            ui.selectable_value(
                &mut dirty_route_type,
                Type::Radial,
                Type::Radial.to_string(),
            );
            ui.selectable_value(
                &mut dirty_route_type,
                Type::ForwardCycle,
                Type::ForwardCycle.to_string(),
            );
            ui.selectable_value(
                &mut dirty_route_type,
                Type::ForwardBackwardsCycle,
                Type::ForwardBackwardsCycle.to_string(),
            );
        });

    if route_type == dirty_route_type {
        return;
    }

    let current_route_points = match &dirty_level_object.route {
        None => vec![],
        Some(ObjectRoute {
            desc: ObjectRouteDesc::Attached(route_point) | ObjectRouteDesc::Radial(route_point),
            ..
        }) => {
            let mut points = Vec::new();
            if let Some(route_point) = route_point {
                points.push(*route_point);
            }
            points
        }
        Some(ObjectRoute {
            desc:
                ObjectRouteDesc::ForwardCycle(route_points)
                | ObjectRouteDesc::ForwardBackwardsCycle(route_points),
            ..
        }) => route_points.clone(),
    };

    match dirty_route_type {
        Type::Stationary => {
            dirty_level_object.route = None;
        }
        Type::Attached => {
            replace_route_desc(
                &mut dirty_level_object.route,
                ObjectRouteDesc::Attached(current_route_points.get(0).cloned()),
            );
        }
        Type::Radial => {
            replace_route_desc(
                &mut dirty_level_object.route,
                ObjectRouteDesc::Radial(current_route_points.get(0).cloned()),
            );
        }
        Type::ForwardCycle => {
            replace_route_desc(
                &mut dirty_level_object.route,
                ObjectRouteDesc::ForwardCycle(current_route_points),
            );
        }
        Type::ForwardBackwardsCycle => {
            replace_route_desc(
                &mut dirty_level_object.route,
                ObjectRouteDesc::ForwardBackwardsCycle(current_route_points),
            );
        }
    }
}

fn replace_route_desc(route: &mut Option<ObjectRoute>, desc: ObjectRouteDesc) {
    if let Some(route) = route {
        route.desc = desc;
    } else {
        *route = Some(ObjectRoute {
            period: DEFAULT_PERIOD,
            start_frame_offset: FrameNumber::new(0),
            desc,
        });
    }
}
