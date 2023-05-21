use crate::{plugins::logging::LogsReceiver, ui::widgets::logs_viewer::LogsViewer};
use bevy::ecs::system::{Local, ResMut, Resource, Res};
use bevy_egui::{egui, EguiContexts};
use crate::ui::debug_ui::DebugUiState;

#[derive(Default, Resource)]
pub struct OccupiedScreenSpace {
    pub bottom: f32,
}

#[derive(Default, Copy, Clone, Eq, PartialEq)]
pub enum BottomPanelTabs {
    #[default]
    Logs,
    Profiler,
}

#[derive(Default)]
pub struct SidePanelsState {
    active_bottom_panel_tab: BottomPanelTabs,
}

pub fn side_panels_ui_system(
    debug_ui_state: Res<DebugUiState>,
    mut side_panels_state: Local<SidePanelsState>,
    mut contexts: EguiContexts,
    mut logs_receiver: ResMut<LogsReceiver>,
    mut occupied_screen_space: ResMut<OccupiedScreenSpace>,
) {
    if !debug_ui_state.show {
        occupied_screen_space.bottom = 0.0;
        return;
    }

    logs_receiver.receive();
    let ctx = contexts.ctx_mut();

    occupied_screen_space.bottom = egui::TopBottomPanel::bottom("bottom_panel")
        .resizable(true)
        .show(ctx, |ui| {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut side_panels_state.active_bottom_panel_tab,
                    BottomPanelTabs::Logs,
                    "Logs",
                );
                ui.selectable_value(
                    &mut side_panels_state.active_bottom_panel_tab,
                    BottomPanelTabs::Profiler,
                    "Profiler",
                );
            });
            ui.separator();

            match side_panels_state.active_bottom_panel_tab {
                BottomPanelTabs::Logs => {
                    LogsViewer::new(logs_receiver.entires()).show(ui);
                }
                BottomPanelTabs::Profiler => {
                    puffin_egui::profiler_ui(ui);
                }
            }
        })
        .response
        .rect
        .height();
}
