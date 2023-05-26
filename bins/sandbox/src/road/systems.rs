use crate::road::{mesh::RoadMesh, ControlPointIndex, Road, RoadModel, SelectedRoad};
use bevy::prelude::*;
use bevy_egui::{egui, egui::Widget, EguiContexts};
use bevy_mod_picking::Selection;

pub fn edit_road(
    mut egui: EguiContexts,
    selected_road: Res<SelectedRoad>,
    mut road_segment_query: Query<(&Children, &mut Road)>,
    mut control_points_query: Query<(
        Entity,
        &ControlPointIndex,
        &mut Transform,
        &Selection,
        &mut Interaction,
    )>,
) {
    let ctx = egui.ctx_mut();

    let Ok((children, mut road)) = road_segment_query.get_mut(**selected_road) else {
        return;
    };

    let mut control_points = children
        .into_iter()
        .filter_map(|child| control_points_query.get(*child).ok())
        .map(
            |(entity, control_point_index, transform, selection, interaction)| {
                (
                    entity,
                    control_point_index.clone(),
                    transform.clone(),
                    *selection,
                    interaction.clone(),
                )
            },
        )
        .collect::<Vec<_>>();
    control_points.sort_by(|(_, a, _, _, _), (_, b, _, _, _)| a.0.cmp(&b.0));

    egui::Window::new("Road").show(ctx, |ui| {
        let mut edge_ring_count = road.edge_ring_count;
        ui.horizontal(|ui| {
            ui.label("edge ring count: ");
            egui::widgets::Slider::new(&mut edge_ring_count, 2..=64)
                .fixed_decimals(0)
                .ui(ui);
        });

        if edge_ring_count != road.edge_ring_count {
            road.edge_ring_count = edge_ring_count;
        }

        ui.label("Control points:");
        for (entity, _, control_point_transform, _selection, _interaction) in &mut control_points {
            ui.horizontal(|ui| {
                ui.label("x:");
                egui::widgets::Slider::new(
                    &mut control_point_transform.translation.x,
                    -15.0..=15.0,
                )
                .ui(ui);
                ui.label("y:");
                egui::widgets::Slider::new(
                    &mut control_point_transform.translation.y,
                    -15.0..=15.0,
                )
                .ui(ui);
                ui.label("z:");
                egui::widgets::Slider::new(
                    &mut control_point_transform.translation.z,
                    -15.0..=15.0,
                )
                .ui(ui);
            });
            if control_point_transform.translation
                != control_points_query
                    .get_component::<Transform>(*entity)
                    .unwrap()
                    .translation
            {
                control_points_query
                    .get_component_mut::<Transform>(*entity)
                    .unwrap()
                    .translation = control_point_transform.translation;
            }
        }
    });
}

pub fn sync_mesh(
    mut meshes: ResMut<Assets<Mesh>>,
    changed_control_points: Query<&Parent, (Changed<Transform>, With<ControlPointIndex>)>,
    changed_roads: Query<Entity, Or<(Changed<Children>, Changed<Road>)>>,
    control_points_query: Query<(&ControlPointIndex, &Transform)>,
    mut road_segments: Query<(&Children, &Road, &mut Handle<Mesh>)>,
) {
    let mut changed_road_entities = Vec::new();
    changed_road_entities.append(&mut changed_control_points.iter().map(|p| p.get()).collect());
    changed_road_entities.append(&mut changed_road_entities.iter().cloned().collect());
    changed_road_entities.append(&mut changed_roads.iter().collect());

    changed_road_entities.sort();
    changed_road_entities.dedup();

    for road_entity in changed_road_entities {
        let Ok((road_segment_children, road, mut mesh_handle)) =
            road_segments.get_mut(road_entity) else {
            continue;
        };
        let mut control_points = road_segment_children
            .into_iter()
            .filter_map(|child| control_points_query.get(*child).ok())
            .collect::<Vec<_>>();
        control_points.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0));
        for i in 0..control_points.len() {
            assert_eq!(control_points[i].0 .0, i);
        }

        let control_points: Vec<_> = control_points
            .iter()
            .map(|(_, transform)| transform.translation)
            .collect();

        *mesh_handle = meshes.set(
            mesh_handle.clone_weak(),
            Mesh::from(RoadMesh {
                edge_ring_count: road.edge_ring_count,
                model: RoadModel { control_points },
            }),
        )
    }
}
