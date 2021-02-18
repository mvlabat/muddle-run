use crate::{ui::debug_ui::DebugUiState, CurrentPlayerNetId};
use bevy::{
    ecs::SystemParam,
    input::{keyboard::KeyboardInput, mouse::MouseButtonInput},
    log,
    prelude::*,
};
use mr_shared_lib::{player::PlayerUpdates, GameTime, COMPONENT_FRAMEBUFFER_LIMIT};

#[derive(Default)]
pub struct TrackInputState {
    pub keys: EventReader<KeyboardInput>,
    pub cursor: EventReader<CursorMoved>,
    pub mouse_button: EventReader<MouseButtonInput>,
}

#[derive(Default)]
pub struct MousePosition(pub Vec2);

#[derive(SystemParam)]
pub struct InputParams<'a> {
    keyboard_input: Res<'a, Input<KeyCode>>,
    ev_keys: Res<'a, Events<KeyboardInput>>,
    ev_cursor: Res<'a, Events<CursorMoved>>,
    ev_mouse_button: Res<'a, Events<MouseButtonInput>>,
}

pub fn track_input_events(
    time: Res<GameTime>,
    mut state: ResMut<TrackInputState>,
    mut mouse_position: ResMut<MousePosition>,
    mut debug_ui_state: ResMut<DebugUiState>,
    mut player_updates: ResMut<PlayerUpdates>,
    current_player_net_id: Res<CurrentPlayerNetId>,
    input: InputParams,
) {
    if input.keyboard_input.just_pressed(KeyCode::Period) {
        debug_ui_state.show = !debug_ui_state.show;
    }

    // Keyboard input.
    if let Some(player_net_id) = current_player_net_id.0 {
        let updates =
            player_updates.get_mut(player_net_id, time.game_frame, COMPONENT_FRAMEBUFFER_LIMIT);
        let mut direction = Vec2::zero();
        if input.keyboard_input.pressed(KeyCode::A) || input.keyboard_input.pressed(KeyCode::Left) {
            direction.x += 1.0;
        }
        if input.keyboard_input.pressed(KeyCode::D) || input.keyboard_input.pressed(KeyCode::Right)
        {
            direction.x -= 1.0;
        }

        if input.keyboard_input.pressed(KeyCode::W) || input.keyboard_input.pressed(KeyCode::Up) {
            direction.y += 1.0;
        }
        if input.keyboard_input.pressed(KeyCode::S) || input.keyboard_input.pressed(KeyCode::Down) {
            direction.y -= 1.0;
        }
        updates.insert(time.game_frame, Some(direction));
    }
    for ev in state.keys.iter(&input.ev_keys) {
        if ev.state.is_pressed() {
            log::trace!("Just pressed key: {:?}", ev.key_code);
        } else {
            log::trace!("Just released key: {:?}", ev.key_code);
        }
    }

    // Absolute cursor position (in window coordinates).
    if let Some(ev) = state.cursor.latest(&input.ev_cursor) {
        mouse_position.0 = ev.position;
    }

    // Mouse buttons.
    for ev in state.mouse_button.iter(&input.ev_mouse_button) {
        if ev.state.is_pressed() {
            log::trace!("Just pressed mouse button: {:?}", ev.button);
        } else {
            log::trace!("Just released mouse button: {:?}", ev.button);
        }
    }
}
