use crate::{
    framebuffer::{FrameNumber, Framebuffer},
    messages::PlayerNetId,
};
use bevy::math::Vec2;
use std::collections::HashMap;

#[derive(Default)]
pub struct PlayerUpdates {
    pub updates: HashMap<PlayerNetId, Framebuffer<Option<Vec2>>>,
}

impl PlayerUpdates {
    pub fn get_mut(
        &mut self,
        player_net_id: PlayerNetId,
        frame_number: FrameNumber,
        default_limit: u16,
    ) -> &mut Framebuffer<Option<Vec2>> {
        self.updates.entry(player_net_id).or_insert_with(|| {
            let mut buffer = Framebuffer::new(frame_number, default_limit);
            buffer.push(Some(Vec2::zero()));
            buffer
        })
    }
}

pub struct Player {
    pub nickname: String,
    pub connected_at: FrameNumber,
}

pub fn random_name() -> String {
    let mut generator = names::Generator::default();
    let name = generator.next().unwrap();
    name.split('-')
        .map(|name_part| {
            let mut chars = name_part.chars().collect::<Vec<_>>();
            chars[0] = chars[0].to_uppercase().next().unwrap();
            chars.into_iter().collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("")
}
