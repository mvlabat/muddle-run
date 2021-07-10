use bevy_egui::egui::{self, Id, Ui};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ListItem<T> {
    pub id: Id,
    pub label: String,
    pub data: T,
    pub sortable: bool,
}

#[derive(Clone, Default, Debug)]
struct DraggedItemData(Option<DraggedItem>);

#[derive(Clone, Debug)]
struct DraggedItem {
    id: Id,
    initial_drag_pos: egui::Pos2,
    initial_rect: egui::Rect,
}

#[derive(Clone, Debug)]
struct SortableListData<T: Clone> {
    list: Vec<ListItem<T>>,
    dragged_item: Option<DraggedItem>,
}

impl<T: Clone> Default for SortableListData<T> {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            dragged_item: None,
        }
    }
}

struct PaintItemJob {
    rect: egui::Rect,
    padding: egui::Vec2,
    item_visuals: egui::style::WidgetVisuals,
    label_galley: std::sync::Arc<egui::epaint::Galley>,
    cross_galley: std::sync::Arc<egui::epaint::Galley>,
    cross_color: egui::Color32,
}

impl PaintItemJob {
    fn paint(self, ui: &mut Ui) {
        let shrank_rect = self.rect.shrink2(self.padding);
        let cross_rect = egui::Rect::from_min_size(
            egui::Pos2::new(
                shrank_rect.max.x - self.cross_galley.size.x - self.padding.x,
                shrank_rect.min.y,
            ),
            self.cross_galley.size,
        );

        let label_cursor = ui
            .layout()
            .align_size_within_rect(self.label_galley.size, shrank_rect)
            .min;

        ui.painter().rect(
            self.rect,
            self.item_visuals.corner_radius,
            self.item_visuals.bg_fill,
            self.item_visuals.bg_stroke,
        );
        ui.painter()
            .galley(label_cursor, self.label_galley, ui.visuals().text_color());
        let cross_cursor = ui
            .layout()
            .align_size_within_rect(self.cross_galley.size, cross_rect)
            .min;
        ui.painter()
            .galley(cross_cursor, self.cross_galley, self.cross_color);
    }
}

pub fn sortable_list<
    T: Clone + Send + Sync + Eq + std::hash::Hash + std::fmt::Debug + 'static,
    I: std::hash::Hash,
>(
    ui: &mut Ui,
    list_id: I,
    list: &mut Vec<ListItem<T>>,
) -> bool {
    let first_unsortable_count = list
        .iter()
        .position(|item| item.sortable)
        .unwrap_or(list.len());
    let last_unsortable_count = list
        .iter()
        .rev()
        .position(|item| item.sortable)
        .unwrap_or(list.len());

    if list.iter().enumerate().any(|(i, item)| {
        !item.sortable && i >= first_unsortable_count && i < list.len() - last_unsortable_count
    }) {
        panic!("Gaps between sortable elements are not allowed");
    }

    let unique_ids = list.iter().map(|i| i.id).collect::<HashSet<_>>();
    if unique_ids.len() != list.len() {
        panic!("Sortable list elements must be unique");
    }

    let list_id = Id::new(list_id);
    let mut memory = ui.memory();
    let sortable_list_data = memory
        .id_data_temp
        .get_mut_or_default::<SortableListData<T>>(list_id);

    let current_list_set = sortable_list_data
        .list
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let list_set = list.iter().cloned().collect::<HashSet<_>>();
    if current_list_set != list_set {
        sortable_list_data.dragged_item = None;
        sortable_list_data.list = list.clone();
    }

    let mut sortable_list_data = sortable_list_data.clone();
    drop(memory);

    let mut list_item_rects = Vec::new();

    let cross_text = "❌";
    let padding = ui.spacing().button_padding;
    let available_width = ui.available_width();

    let cross_galley = ui
        .fonts()
        .layout_no_wrap(egui::TextStyle::Button, cross_text.to_owned());
    let total_extra = egui::Vec2::new(cross_galley.size.x, 0.0)
        + padding * 2.0
        + egui::Vec2::new(padding.x, 0.0) * 2.0;
    let desired_size = egui::Vec2::new(available_width, cross_galley.size.y + total_extra.y);

    let mut item_to_remove: Option<usize> = None;
    let mut draggable_current_index: Option<usize> = None;
    let mut delayed_paint_job: Option<PaintItemJob> = None;

    for (i, list_item) in sortable_list_data.list.iter().enumerate() {
        let (id, mut rect) = ui.allocate_space(desired_size);
        list_item_rects.push(rect);

        let mut is_dragged_item = sortable_list_data
            .dragged_item
            .as_ref()
            .map_or(false, |item| item.id == list_item.id);

        if is_dragged_item {
            let dragged_item = sortable_list_data.dragged_item.as_ref().unwrap();
            let rect_delta = dragged_item.initial_rect.min - rect.min;
            let delta = ui.input().pointer.interact_pos().unwrap() - dragged_item.initial_drag_pos
                + rect_delta;
            rect.min += delta;
            rect.max += delta;
        }

        if ui.clip_rect().intersects(rect) {
            let sense = if list_item.sortable {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            };

            let label_galley = ui.fonts().layout_multiline(
                egui::TextStyle::Button,
                list_item.label.clone(),
                available_width - total_extra.x,
            );

            let shrank_rect = rect.shrink2(padding);
            let cross_rect = egui::Rect::from_min_size(
                egui::Pos2::new(
                    shrank_rect.max.x - cross_galley.size.x - padding.x,
                    shrank_rect.min.y,
                ),
                cross_galley.size,
            );
            let cross_response = ui.interact(cross_rect, list_item.id.with("remove"), sense);
            if cross_response.clicked() {
                item_to_remove = Some(i);
            }

            let item_response = ui.interact(rect, id, sense);

            let interacting_with_cross =
                cross_response.dragged() || cross_response.hovered() || cross_response.clicked();

            if !interacting_with_cross
                && item_response.dragged()
                && sortable_list_data.dragged_item.is_none()
            {
                is_dragged_item = true;
                sortable_list_data.dragged_item = Some(DraggedItem {
                    id: list_item.id,
                    initial_drag_pos: ui.input().pointer.interact_pos().unwrap(),
                    initial_rect: rect,
                });
            }

            let item_visuals = if !list_item.sortable {
                ui.style().visuals.widgets.noninteractive
            } else if sortable_list_data.dragged_item.is_some() {
                if is_dragged_item {
                    ui.style().visuals.widgets.active
                } else {
                    ui.style().visuals.widgets.inactive
                }
            } else {
                *ui.style().interact(&item_response)
            };

            let cross_color = if !list_item.sortable {
                egui::Color32::from_rgb(80, 80, 80)
            } else if cross_response.hovered() {
                egui::Color32::from_rgb(200, 20, 20)
            } else {
                egui::Color32::from_rgb(140, 30, 30)
            };

            let paint_item_job = PaintItemJob {
                rect,
                padding,
                item_visuals,
                label_galley,
                cross_galley: cross_galley.clone(),
                cross_color,
            };
            if is_dragged_item {
                draggable_current_index = Some(i);
                delayed_paint_job = Some(paint_item_job);
            } else {
                paint_item_job.paint(ui);
            }

            if !interacting_with_cross && item_response.hovered() && list_item.sortable {
                ui.output().cursor_icon = egui::CursorIcon::Grab;
            }
        }
    }

    if let Some(i) = item_to_remove {
        sortable_list_data.list.remove(i);
    }

    if !ui.memory().is_anything_being_dragged() {
        sortable_list_data.dragged_item = None;
    }

    if sortable_list_data.dragged_item.is_some() {
        if let Some(delayed_paint_job) = delayed_paint_job {
            let draggable_new_index = list_item_rects
                .iter()
                .position(|item_rect| item_rect.contains(delayed_paint_job.rect.center()));
            if let Some((new_i, old_i)) = draggable_new_index.zip(draggable_current_index) {
                if sortable_list_data.list[new_i].sortable {
                    let item = sortable_list_data.list.remove(old_i);
                    sortable_list_data.list.insert(new_i, item);
                }
            }

            delayed_paint_job.paint(ui);
        }

        ui.output().cursor_icon = egui::CursorIcon::Grabbing;
    }

    ui.separator();

    let mut edited = false;
    if sortable_list_data.dragged_item.is_none() && *list != sortable_list_data.list {
        *list = sortable_list_data.list.clone();
        edited = true;
    }

    *ui.memory()
        .id_data_temp
        .get_mut::<SortableListData<T>>(&list_id)
        .unwrap() = sortable_list_data;

    edited
}
