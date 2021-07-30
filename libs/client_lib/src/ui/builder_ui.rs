use crate::{
    helpers::{MouseEntityPicker, PlayerParams},
    input::{LevelObjectRequestsQueue, MouseScreenPosition, MouseWorldPosition},
    ui::widgets::sortable::{sortable_list, ListItem},
    LevelObjectCorrelations,
};
use bevy::{
    ecs::{
        entity::Entity,
        schedule::{ParallelSystemDescriptorCoercion, ShouldRun, SystemSet},
        system::{IntoSystem, Local, Query, Res, ResMut, SystemParam},
    },
    input::{mouse::MouseButton, Input},
    log,
    math::Vec2,
    transform::components::Transform,
    utils::HashMap,
};
use bevy_egui::{
    egui::{self, Ui, Widget},
    EguiContext,
};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        components::{
            LevelObjectLabel, LevelObjectStaticGhost, LevelObjectStaticGhostParent, Spawned,
        },
        level::{LevelObject, LevelObjectDesc, LevelState, ObjectRoute, ObjectRouteDesc},
        level_objects::{CubeDesc, PlaneDesc, PlaneFormDesc, RoutePointDesc},
    },
    messages::{EntityNetId, SpawnLevelObjectRequest, SpawnLevelObjectRequestBody},
    net::MessageId,
    player::PlayerRole,
    registry::EntityRegistry,
    simulations_per_second, GameTime,
};

pub const DEFAULT_PLANE_CIRCLE_RADIUS: f32 = 10.0;
pub const DEFAULT_PLANE_RECTANGLE_SIZE: [f32; 2] = [10.0, 10.0];
pub const DEFAULT_PLANE_CONCAVE_POINTS: &[[f32; 2]] = &[
    [-8.0, -5.0],
    [8.0, -5.0],
    [10.0, 5.0],
    [0.0, 3.50],
    [-10.0, 5.0],
];

pub fn default_period() -> FrameNumber {
    FrameNumber::new(simulations_per_second() * 10)
}

#[derive(Default, Clone)]
pub struct EditedLevelObject {
    pub object: Option<(Entity, LevelObject)>,
    pub dragged_control_point_index: Option<usize>,
    pub is_being_placed: bool,
    pub is_being_dragged: bool,
}

impl EditedLevelObject {
    pub fn deselect(&mut self) {
        self.object = None;
        self.dragged_control_point_index = None;
        self.is_being_dragged = false;
        self.is_being_placed = false;
    }
}

pub type LevelObjectsQuery<'a> = Query<
    'a,
    (
        Entity,
        &'static LevelObjectLabel,
        &'static Transform,
        &'static LevelObjectStaticGhostParent,
        &'static Spawned,
    ),
>;

#[derive(SystemParam)]
pub struct LevelObjects<'a> {
    time: Res<'a, GameTime>,
    pending_correlation: Local<'a, Option<MessageId>>,
    edited_level_object: ResMut<'a, EditedLevelObject>,
    level_state: Res<'a, LevelState>,
    entity_registry: Res<'a, EntityRegistry<EntityNetId>>,
    query: LevelObjectsQuery<'a>,
    ghosts_query: Query<'a, (&'static LevelObjectStaticGhost, &'static Transform)>,
}

#[derive(SystemParam)]
pub struct MouseInput<'a> {
    pub mouse_screen_position: Res<'a, MouseScreenPosition>,
    pub mouse_world_position: Res<'a, MouseWorldPosition>,
    pub mouse_entity_picker: MouseEntityPicker<'a>,
    pub mouse_button_input: Res<'a, Input<MouseButton>>,
}

#[derive(Default)]
pub struct BuilderUiState {
    select_edited_level_object_filter: String,
    route_point_filter: String,
}

pub fn builder_system_set() -> SystemSet {
    SystemSet::new()
        .with_run_criteria(builder_run_criteria.system())
        .with_system(builder_ui.system().label("ui"))
        .with_system(process_builder_mouse_input.system().after("ui"))
}

pub fn builder_run_criteria(
    player_params: PlayerParams,
    mut edited_level_object: ResMut<EditedLevelObject>,
) -> ShouldRun {
    puffin::profile_function!();
    let player = match player_params.current_player() {
        Some(player) => player,
        None => {
            edited_level_object.deselect();
            return ShouldRun::No;
        }
    };
    if !matches!(player.role, PlayerRole::Builder) {
        edited_level_object.deselect();
        return ShouldRun::No;
    }
    ShouldRun::Yes
}

pub fn process_builder_mouse_input(
    egui_context: ResMut<EguiContext>,
    mut mouse_input: MouseInput,
    mut level_objects: LevelObjects,
    mut level_object_requests: ResMut<LevelObjectRequestsQueue>,
    // Screen coordinates at where the dragging started.
    mut dragging_start: Local<Option<Vec2>>,
) {
    puffin::profile_function!();
    // If we have a newly placed object, move it with a cursor, until left mouse button is clicked.
    if let EditedLevelObject {
        object: Some((_, level_object)),
        is_being_placed,
        is_being_dragged,
        ..
    } = &mut *level_objects.edited_level_object
    {
        if *is_being_placed || *is_being_dragged {
            let object_position = level_object
                .desc
                .position_mut()
                .expect("Objects without positions aren't supported yet");
            if (*object_position - mouse_input.mouse_world_position.0).length_squared()
                > f32::EPSILON
            {
                *object_position = mouse_input.mouse_world_position.0;
                level_object_requests.update_requests.push(LevelObject {
                    net_id: level_object.net_id,
                    label: level_object.label.clone(),
                    desc: level_object.desc.clone(),
                    route: level_object.route.clone(),
                });
            }
        }

        if *is_being_placed
            && mouse_input
                .mouse_button_input
                .just_pressed(MouseButton::Left)
            && !egui_context.ctx().is_pointer_over_area()
        {
            *is_being_placed = false;
        }
    }

    if level_objects.edited_level_object.object.is_none()
        || !level_objects.edited_level_object.is_being_placed
            && !level_objects.edited_level_object.is_being_dragged
            && level_objects
                .edited_level_object
                .dragged_control_point_index
                .is_none()
    {
        // Picking a level object with a mouse.
        if !egui_context.ctx().is_pointer_over_area() {
            mouse_input.mouse_entity_picker.pick_entity();
        }
        let mut is_ghost = false;
        if let Some((entity, _, edited_level_object)) = mouse_input
            .mouse_entity_picker
            .take_picked_entity()
            .and_then(|entity| {
                // Checking whether we've clicked a ghost.
                if let Ok(LevelObjectStaticGhost(ghost_parent)) =
                    level_objects
                        .ghosts_query
                        .get_component::<LevelObjectStaticGhost>(entity)
                {
                    is_ghost = true;
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
            let is_static = {
                let (_, _, transform, LevelObjectStaticGhostParent(ghost_entity), _) =
                    level_objects.query.get(entity).unwrap();
                let ghost_transform = level_objects
                    .ghosts_query
                    .get_component::<Transform>(*ghost_entity)
                    .unwrap();
                (transform.translation.x - ghost_transform.translation.x).abs() < f32::EPSILON
                    && (transform.translation.y - ghost_transform.translation.y).abs()
                        < f32::EPSILON
            };
            let is_draggable = edited_level_object.desc.is_movable_with_mouse()
                && (edited_level_object.route.is_none() || is_ghost || is_static);
            if is_draggable {
                *dragging_start = Some(mouse_input.mouse_screen_position.0);
            }
            // We don't reset edited state if the clicked object is the same.
            if !matches!(level_objects.edited_level_object.object, Some((picked_entity, _)) if picked_entity == entity)
            {
                level_objects.edited_level_object.object = Some((entity, edited_level_object));
                *level_objects.pending_correlation = None;
            }
        }
    }

    if let Some(dragging_start_position) = *dragging_start {
        if mouse_input.mouse_button_input.pressed(MouseButton::Left) {
            let dragging_threshold_squared = 100.0;
            if (mouse_input.mouse_screen_position.0 - dragging_start_position).length_squared()
                > dragging_threshold_squared
            {
                level_objects.edited_level_object.is_being_dragged = true;
            }
        } else {
            *dragging_start = None;
            level_objects.edited_level_object.is_being_dragged = false;
        }
    }
}

pub fn builder_ui(
    egui_context: ResMut<EguiContext>,
    mut builder_ui_state: Local<BuilderUiState>,
    mouse_input: MouseInput,
    mut level_object_correlations: ResMut<LevelObjectCorrelations>,
    mut level_object_requests: ResMut<LevelObjectRequestsQueue>,
    mut level_objects: LevelObjects,
) {
    puffin::profile_function!();
    let ctx = egui_context.ctx();

    // Picking a level object if we received a confirmation from the server about an object created
    // by us.
    if let Some(correlation_id) = *level_objects.pending_correlation {
        if let Some(entity_net_id) = level_object_correlations.query(correlation_id) {
            level_objects.edited_level_object.object =
                level_objects.entity_registry.get_entity(entity_net_id).zip(
                    level_objects
                        .level_state
                        .objects
                        .get(&entity_net_id)
                        .cloned(),
                );
            if let Some((_, edited_level_object)) = &level_objects.edited_level_object.object {
                if edited_level_object.desc.is_movable_with_mouse() {
                    level_objects.edited_level_object.is_being_placed = true;
                }
            } else {
                log::error!("Level object {} isn't registered", entity_net_id.0);
            }
            *level_objects.pending_correlation = None;
        }
    }

    if level_objects.edited_level_object.object.is_some() {
        // When an object is updated, it may get re-spawned as a new entity. We need to update
        // the picked entity in such a case. Despawns may happen as well.
        let edited_object_net_id = level_objects
            .edited_level_object
            .object
            .as_ref()
            .unwrap()
            .1
            .net_id;
        if let Some(level_object_entity) = level_objects
            .entity_registry
            .get_entity(edited_object_net_id)
        {
            if level_objects
                .query
                .get_component::<Spawned>(level_object_entity)
                .unwrap()
                .is_spawned(level_objects.time.frame_number)
            {
                let (entity, level_object) =
                    level_objects.edited_level_object.object.as_mut().unwrap();
                if *entity != level_object_entity {
                    *entity = level_object_entity;
                    if !ctx.is_using_pointer() {
                        *level_object = level_objects
                            .level_state
                            .objects
                            .get(&edited_object_net_id)
                            .cloned()
                            .unwrap();
                    }
                }
            } else {
                level_objects.edited_level_object.deselect();
            }
        } else {
            level_objects.edited_level_object.deselect();
        }

        // We don't want to display the builder UI if the object is being placed.
        // Dragging is ok though.
        if level_objects.edited_level_object.is_being_placed {
            return;
        }
    }

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
                            position: mouse_input.mouse_world_position.0,
                            form_desc: PlaneFormDesc::Rectangle {
                                size: DEFAULT_PLANE_RECTANGLE_SIZE.into(),
                            },
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
                            position: mouse_input.mouse_world_position.0,
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
                                position: mouse_input.mouse_world_position.0,
                            },
                        )),
                    });
            }
        });

        ui.separator();
        ui.collapsing("Select object to edit", |ui| {
            if let Some(entity) = level_objects_filter(
                ui,
                &mut builder_ui_state.select_edited_level_object_filter,
                &level_objects.time,
                &level_objects.query,
            ) {
                let entity_net_id = level_objects.entity_registry.get_id(entity).unwrap();
                let level_object = level_objects
                    .level_state
                    .objects
                    .get(&entity_net_id)
                    .unwrap()
                    .clone();
                level_objects.edited_level_object.object = Some((entity, level_object));
            }
        });

        if let Some((_, level_object)) = level_objects.edited_level_object.object.clone() {
            let mut dirty_level_object = level_object.clone();
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

                let (_, edited_level_object) =
                    level_objects.edited_level_object.object.as_mut().unwrap();
                *edited_level_object = dirty_level_object;
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
    egui::Grid::new("editing_edited_level_object.object")
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
                LevelObjectDesc::Cube(CubeDesc { size, .. }) => {
                    ui.label("Size");
                    ui.add(egui::widgets::DragValue::new(size).speed(0.01));
                    ui.end_row();
                }
                LevelObjectDesc::Plane(PlaneDesc { form_desc, .. }) => {
                    plane_form(ui, form_desc);
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
                            route.period = default_period()
                                .max(route.start_frame_offset + FrameNumber::new(1));
                        }

                        ui.label("Period (frames)");
                        ui.add(
                            egui::widgets::DragValue::new(&mut route.period)
                                .speed(0.1)
                                .clamp_range(
                                    simulations_per_second()
                                        .max(route.start_frame_offset.value() + 1)
                                        ..=simulations_per_second() * 60,
                                ),
                        );
                        ui.end_row();

                        ui.label("Period (second)");
                        ui.label(format!(
                            "{:.2}",
                            route.period.value() as f32 / simulations_per_second() as f32
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

fn plane_form(ui: &mut egui::Ui, dirty_plane_form_desc: &mut PlaneFormDesc) {
    ui.label("Form type");
    plane_form_type(ui, dirty_plane_form_desc);
    ui.end_row();

    match dirty_plane_form_desc {
        PlaneFormDesc::Circle { radius } => {
            ui.label("Radius");
            ui.add(
                egui::widgets::DragValue::new(radius)
                    .speed(0.01)
                    .clamp_range(1.0..=f32::MAX),
            );
            ui.end_row();
        }
        PlaneFormDesc::Rectangle { size } => {
            ui.label("Size");
            ui.horizontal(|ui| {
                ui.label("Width:");
                ui.add(
                    egui::widgets::DragValue::new(&mut size.x)
                        .speed(0.01)
                        .clamp_range(1.0..=f32::MAX),
                );
                ui.label("Height:");
                ui.add(
                    egui::widgets::DragValue::new(&mut size.y)
                        .speed(0.01)
                        .clamp_range(1.0..=f32::MAX),
                );
            });
            ui.end_row();
        }
        PlaneFormDesc::Concave { points } => {
            ui.label("Points");
            ui.vertical(|ui| {
                ui.group(|ui| {
                    egui::ScrollArea::from_max_height(200.0).show(ui, |ui| {
                        let removing_enabled = points.len() > 3;
                        let mut point_to_remove = None;
                        for (i, point) in points.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label("X:");
                                ui.add(egui::widgets::DragValue::new(&mut point.x).speed(0.1));
                                ui.label("Y:");
                                ui.add(egui::widgets::DragValue::new(&mut point.y).speed(0.1));
                                if egui::Button::new("❌")
                                    .enabled(removing_enabled)
                                    .ui(ui)
                                    .clicked()
                                {
                                    point_to_remove = Some(i);
                                }
                            });
                        }
                        if let Some(point_to_remove) = point_to_remove {
                            points.remove(point_to_remove);
                        }
                    });
                    if ui.button("Add").clicked() {
                        points.push(Vec2::new(1.0, 1.0));
                    }
                });
            });
            ui.end_row();
        }
    }
}

fn plane_form_type(ui: &mut egui::Ui, dirty_plane_form_desc: &mut PlaneFormDesc) {
    #[derive(Copy, Clone, PartialEq, Debug)]
    enum Type {
        Circle,
        Rectangle,
        Concave,
    }

    impl std::fmt::Display for Type {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Type::Circle => write!(f, "Circle"),
                Type::Rectangle => write!(f, "Rectangle"),
                Type::Concave => write!(f, "Concave"),
            }
        }
    }

    let plane_form_type = match dirty_plane_form_desc {
        PlaneFormDesc::Circle { .. } => Type::Circle,
        PlaneFormDesc::Rectangle { .. } => Type::Rectangle,
        PlaneFormDesc::Concave { .. } => Type::Concave,
    };
    let mut dirty_plane_form_type = plane_form_type;

    egui::containers::ComboBox::from_id_source("plane_form")
        .width(200.0)
        .selected_text(plane_form_type.to_string())
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut dirty_plane_form_type,
                Type::Circle,
                Type::Circle.to_string(),
            );
            ui.selectable_value(
                &mut dirty_plane_form_type,
                Type::Rectangle,
                Type::Rectangle.to_string(),
            );
            ui.selectable_value(
                &mut dirty_plane_form_type,
                Type::Concave,
                Type::Concave.to_string(),
            );
        });

    if plane_form_type == dirty_plane_form_type {
        return;
    }

    *dirty_plane_form_desc = match dirty_plane_form_type {
        Type::Circle => PlaneFormDesc::Circle {
            radius: DEFAULT_PLANE_CIRCLE_RADIUS,
        },
        Type::Rectangle => PlaneFormDesc::Rectangle {
            size: DEFAULT_PLANE_RECTANGLE_SIZE.into(),
        },
        Type::Concave => PlaneFormDesc::Concave {
            points: DEFAULT_PLANE_CONCAVE_POINTS
                .iter()
                .map(|line| (*line).into())
                .collect(),
        },
    };
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
                let mut duplicate_counts = HashMap::default();
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
        if let Some(entity) = level_objects_filter(
            ui,
            &mut builder_ui_state.route_point_filter,
            &level_objects.time,
            &level_objects.query,
        ) {
            let selected_entity_net_id = level_objects
                .entity_registry
                .get_id(entity)
                .expect("Expected a registered level object");
            match &mut dirty_level_object_route.desc {
                ObjectRouteDesc::Attached(route_point) | ObjectRouteDesc::Radial(route_point) => {
                    *route_point = Some(selected_entity_net_id);
                }
                ObjectRouteDesc::ForwardCycle(route_points)
                | ObjectRouteDesc::ForwardBackwardsCycle(route_points) => {
                    route_points.push(selected_entity_net_id);
                }
            }
        }
    }
}

fn level_objects_filter(
    ui: &mut Ui,
    filter: &mut String,
    time: &GameTime,
    objects_query: &LevelObjectsQuery,
) -> Option<Entity> {
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.text_edit_singleline(filter);
        if ui.button("❌").clicked() {
            *filter = String::new();
        }
    });
    let mut result = None;
    egui::ScrollArea::auto_sized().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (entity, label, _, _, spawned) in objects_query.iter() {
                if !spawned.is_spawned(time.frame_number) {
                    continue;
                }

                if !label.0.to_lowercase().contains(&filter.to_lowercase()) {
                    continue;
                }

                if ui.button(&label.0).clicked() {
                    result = Some(entity);
                }
            }
        });
    });
    result
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

    egui::containers::ComboBox::from_id_source("route_type")
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
            period: default_period(),
            start_frame_offset: FrameNumber::new(0),
            desc,
        });
    }
}
