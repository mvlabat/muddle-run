use crate::ui::debug_ui::DebugUiState;
use bevy::{
    input::{keyboard::KeyboardInput, mouse::MouseButtonInput},
    log,
    prelude::*,
};

#[derive(Default)]
pub struct TrackInputState {
    pub keys: EventReader<KeyboardInput>,
    pub cursor: EventReader<CursorMoved>,
    pub mouse_button: EventReader<MouseButtonInput>,
}

#[derive(Default)]
pub struct MousePosition(pub Vec2);

pub fn track_input_events(
    mut state: ResMut<TrackInputState>,
    mut mouse_position: ResMut<MousePosition>,
    mut debug_ui_state: ResMut<DebugUiState>,
    keyboard_input: Res<Input<KeyCode>>,
    ev_keys: Res<Events<KeyboardInput>>,
    ev_cursor: Res<Events<CursorMoved>>,
    ev_mouse_button: Res<Events<MouseButtonInput>>,
) {
    if keyboard_input.just_pressed(KeyCode::Period) {
        debug_ui_state.show = !debug_ui_state.show;
    }

    // Keyboard input.
    for ev in state.keys.iter(&ev_keys) {
        if ev.state.is_pressed() {
            log::trace!("Just pressed key: {:?}", ev.key_code);
        } else {
            log::trace!("Just released key: {:?}", ev.key_code);
        }
    }

    // Absolute cursor position (in window coordinates).
    if let Some(ev) = state.cursor.latest(&ev_cursor) {
        mouse_position.0 = ev.position;
    }

    // Mouse buttons.
    for ev in state.mouse_button.iter(&ev_mouse_button) {
        if ev.state.is_pressed() {
            log::trace!("Just pressed mouse button: {:?}", ev.button);
        } else {
            log::trace!("Just released mouse button: {:?}", ev.button);
        }
    }
}
