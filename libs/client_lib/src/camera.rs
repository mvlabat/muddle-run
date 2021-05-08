use crate::{CurrentPlayerNetId, MainCameraEntity};
use bevy::prelude::*;
use mr_shared_lib::{player::PlayerUpdates, GameTime, COMPONENT_FRAMEBUFFER_LIMIT};

pub const CAMERA_OFFSET: (f32, f32, f32) = (5.0, 10.0, -14.0);

pub fn camera_follow_player(
    time: Res<GameTime>,
    mut player_updates: ResMut<PlayerUpdates>,
    current_player_net_id: Res<CurrentPlayerNetId>,
    main_camera_entity: Res<MainCameraEntity>,
    mut cameras: Query<(&mut Transform, &bevy::render::camera::Camera)>,
) {
    // Get the player's position and update the camera accordingly.
    if let Some(player_net_id) = current_player_net_id.0 {
        let position_updates = player_updates.get_position_mut(
            player_net_id,
            time.frame_number,
            COMPONENT_FRAMEBUFFER_LIMIT,
        );

        if let Some(position) = position_updates.last().and_then(|v| v.as_ref()) {
            let (mut camera_transform, _camera) = cameras
                .get_mut(main_camera_entity.0)
                .expect("expected a main camera");

            camera_transform.translation = Vec3::new(
                CAMERA_OFFSET.0 + position.x,
                CAMERA_OFFSET.1,
                CAMERA_OFFSET.2 + position.y,
            );
        }
    }
}
