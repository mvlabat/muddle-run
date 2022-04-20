use crate::ui::without_item_spacing;
use bevy_egui::egui;

pub struct MenuListItem<Secondary, Collapsing> {
    title: String,
    id: egui::Id,
    is_selected: bool,
    image_widget: Option<Box<dyn FnOnce(&mut egui::Ui)>>,
    secondary: Secondary,
    collapsing: Collapsing,
}

#[derive(Clone)]
pub struct MenuListItemState {
    height: f32,
}

impl MenuListItemState {
    fn animated_height(&self, ctx: &egui::Context, id: egui::Id, selected: bool) -> f32 {
        egui::remap_clamp(
            ctx.animate_bool(id.with("height"), selected),
            0.0..=1.0,
            COLLAPSED_HEIGHT..=self.height,
        )
    }
}

pub struct MenuListItemResponse<SecondaryR, CollapsingR> {
    pub item: egui::Response,
    pub secondary: SecondaryR,
    pub collapsing: Option<CollapsingR>,
}

type DrawNothing = fn(ui: &mut egui::Ui) -> ();

impl MenuListItem<DrawNothing, DrawNothing> {
    pub fn new(title: impl ToString) -> Self {
        MenuListItem {
            title: title.to_string(),
            id: egui::Id::new(title.to_string()),
            is_selected: false,
            image_widget: None,
            secondary: |_| {},
            collapsing: |_| {},
        }
    }
}

const COLLAPSED_HEIGHT: f32 = 60.0;
const IMAGE_SIZE: egui::Vec2 = egui::Vec2::splat(40.0);

impl<Secondary, Collapsing, SecondaryR, CollapsingR> MenuListItem<Secondary, Collapsing>
where
    Secondary: FnOnce(&mut egui::Ui) -> SecondaryR,
    Collapsing: FnOnce(&mut egui::Ui) -> CollapsingR,
{
    pub fn with_id(mut self, id: impl std::hash::Hash) -> Self {
        self.id = egui::Id::new(id);
        self
    }

    pub fn selected(mut self, is_selected: bool) -> Self {
        self.is_selected = is_selected;
        self
    }

    pub fn image_widget(mut self, image_widget: impl FnOnce(&mut egui::Ui) + 'static) -> Self {
        self.image_widget = Some(Box::new(image_widget));
        self
    }

    pub fn secondary_widget<NewSecondary: FnOnce(&mut egui::Ui) -> R, R>(
        self,
        widget: NewSecondary,
    ) -> MenuListItem<NewSecondary, Collapsing> {
        MenuListItem {
            title: self.title,
            id: self.id,
            is_selected: self.is_selected,
            image_widget: self.image_widget,
            secondary: widget,
            collapsing: self.collapsing,
        }
    }

    pub fn collapsing_widget<NewCollapsing: FnOnce(&mut egui::Ui) -> R, R>(
        self,
        widget: NewCollapsing,
    ) -> MenuListItem<Secondary, NewCollapsing> {
        MenuListItem {
            title: self.title,
            id: self.id,
            is_selected: self.is_selected,
            image_widget: self.image_widget,
            secondary: self.secondary,
            collapsing: widget,
        }
    }

    pub fn show(self, ui: &mut egui::Ui) -> MenuListItemResponse<SecondaryR, CollapsingR> {
        let padding = egui::Vec2::new(10.0, 8.0);

        let (outer_rect, response) = without_item_spacing(ui, |ui| {
            ui.allocate_exact_size(
                egui::Vec2::new(ui.max_rect().width(), COLLAPSED_HEIGHT),
                egui::Sense::click(),
            )
        });

        let fill = if self.is_selected {
            Some(ui.style().visuals.extreme_bg_color)
        } else if response.hovered() {
            Some(ui.style().visuals.faint_bg_color)
        } else {
            Some(ui.style().visuals.window_fill())
        };

        let ctx = ui.ctx();
        let item_height = ctx.memory().data.get_temp::<MenuListItemState>(self.id);

        let item_height = item_height.map_or_else(
            || {
                let state = MenuListItemState {
                    height: COLLAPSED_HEIGHT,
                };
                let height = state.animated_height(ctx, self.id, self.is_selected);
                ctx.memory().data.insert_temp(self.id, state);
                height
            },
            |state| state.animated_height(ctx, self.id, self.is_selected),
        );

        if let Some(fill) = fill {
            let filled_rect = egui::Rect::from_min_size(
                outer_rect.min,
                egui::Vec2::new(ui.max_rect().width(), item_height),
            );
            ui.painter().rect_filled(filled_rect, 0.0, fill);
        }

        let item_widgets_min = if let Some(image_widget) = self.image_widget {
            let image_rect = egui::Rect::from_min_size(outer_rect.min + padding, IMAGE_SIZE);
            ui.painter()
                .rect_stroke(image_rect, 0.0, ui.style().visuals.window_stroke());
            let mut image_ui = ui.child_ui(image_rect, *ui.layout());
            image_widget(&mut image_ui);
            image_rect.right_top() + egui::Vec2::new(padding.x, 0.0)
        } else {
            outer_rect.min + padding
        };

        let mut secondary_ui = ui.child_ui(
            egui::Rect::from_min_max(item_widgets_min, outer_rect.max - padding),
            *ui.layout(),
        );
        secondary_ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
        secondary_ui.label(self.title);
        secondary_ui.style_mut().override_text_style = None;
        let secondary = (self.secondary)(&mut secondary_ui);

        let mut collapsing_ui = ui.child_ui(
            egui::Rect::from_min_max(
                outer_rect.left_bottom() + egui::Vec2::new(padding.x, -padding.y),
                outer_rect.right_bottom() - egui::Vec2::new(padding.x, -padding.y),
            ),
            *ui.layout(),
        );
        let (outer_rect, response, actual_collapsing_height, collapsing) =
            if self.is_selected || item_height > COLLAPSED_HEIGHT {
                let width = outer_rect.width();
                let height = item_height - COLLAPSED_HEIGHT;
                collapsing_ui.set_clip_rect(egui::Rect::from_min_size(
                    collapsing_ui.min_rect().min,
                    egui::Vec2::new(width, height),
                ));
                let collapsing = (self.collapsing)(&mut collapsing_ui);
                let actual_collapsing_height = if collapsing_ui.min_size().y > 0.0 {
                    collapsing_ui.min_size().y + padding.y
                } else {
                    0.0
                };
                let (added_outer_rect, added_response) = without_item_spacing(ui, |ui| {
                    ui.allocate_exact_size(egui::Vec2::new(width, height), egui::Sense::click())
                });
                (
                    outer_rect.union(added_outer_rect),
                    response.union(added_response),
                    Some(actual_collapsing_height),
                    Some(collapsing),
                )
            } else {
                (outer_rect, response, None, None)
            };

        ui.painter().line_segment(
            [
                egui::Pos2::new(outer_rect.min.x, outer_rect.max.y),
                egui::Pos2::new(outer_rect.max.x, outer_rect.max.y),
            ],
            ui.style().visuals.window_stroke(),
        );

        if let Some(actual_collapsing_height) = actual_collapsing_height {
            ui.ctx().memory().data.insert_temp(
                self.id,
                MenuListItemState {
                    height: COLLAPSED_HEIGHT + actual_collapsing_height,
                },
            );
        };

        let mut ctx_output = ui.ctx().output();
        if response.hovered() && ctx_output.cursor_icon == egui::CursorIcon::Default {
            ctx_output.cursor_icon = egui::CursorIcon::PointingHand;
        }

        MenuListItemResponse {
            item: response,
            secondary,
            collapsing,
        }
    }
}

pub struct PanelButton {
    widget: egui::Button,
    enabled: bool,
    on_hover_text: Option<egui::WidgetText>,
    on_disabled_hover_text: Option<egui::WidgetText>,
}

impl PanelButton {
    pub fn new(widget: egui::Button) -> Self {
        Self {
            widget,
            enabled: true,
            on_hover_text: None,
            on_disabled_hover_text: None,
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn on_hover_text(mut self, on_hover_text: impl Into<egui::WidgetText>) -> Self {
        self.on_hover_text = Some(on_hover_text.into());
        self
    }

    pub fn on_disabled_hover_text(mut self, on_hover_text: impl Into<egui::WidgetText>) -> Self {
        self.on_disabled_hover_text = Some(on_hover_text.into());
        self
    }
}

pub fn button_panel<const C: usize>(
    ui: &mut egui::Ui,
    button_width: f32,
    buttons: [PanelButton; C],
) -> [egui::Response; C] {
    let button_size = egui::Vec2::new(button_width, 30.0);
    let margin = 10.0;

    let (outer_rect, _) = without_item_spacing(ui, |ui| {
        ui.allocate_exact_size(
            egui::Vec2::new(
                ui.available_size_before_wrap().x,
                button_size.y + margin * 2.0,
            ),
            egui::Sense::hover(),
        )
    });

    // Button coordinates.
    let offset_x_step = outer_rect.size().x / (buttons.len() + 1) as f32;
    let start_pos = egui::Vec2::new(outer_rect.min.x + offset_x_step, outer_rect.center().y);
    let mut i = 0;
    buttons.map(|button| {
        let response = without_item_spacing(ui, |ui| {
            ui.add_enabled_ui(button.enabled, |ui| {
                let mut response = ui.put(
                    egui::Rect::from_min_size(
                        start_pos.to_pos2() + egui::Vec2::new(offset_x_step * i as f32, 0.0)
                            - button_size / 2.0,
                        button_size,
                    ),
                    button.widget,
                );
                if let Some(on_hover_text) = button.on_hover_text {
                    response = response.on_hover_text(on_hover_text);
                }
                if let Some(on_disabled_hover_text) = button.on_disabled_hover_text {
                    response = response.on_disabled_hover_text(on_disabled_hover_text);
                }
                response
            })
        });
        i += 1;
        response.inner
    })
}
