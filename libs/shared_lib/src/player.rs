use crate::{
    framebuffer::{FrameNumber, Framebuffer},
    messages::PlayerNetId,
};
use bevy::math::Vec2;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct PlayerUpdates {
    pub direction: HashMap<PlayerNetId, Framebuffer<Option<PlayerDirectionUpdate>>>,
    /// Is supposed to be filled and used only by clients, as it contains authoritative updates.
    pub position: HashMap<PlayerNetId, Framebuffer<Option<Vec2>>>,
}

#[derive(Debug, PartialEq)]
pub struct PlayerDirectionUpdate {
    pub direction: Vec2,
    pub is_processed_client_input: Option<bool>,
}

impl PlayerUpdates {
    pub fn get_direction_mut(
        &mut self,
        player_net_id: PlayerNetId,
        frame_number: FrameNumber,
        default_limit: u16,
    ) -> &mut Framebuffer<Option<PlayerDirectionUpdate>> {
        self.direction.entry(player_net_id).or_insert_with(|| {
            let mut buffer = Framebuffer::new(frame_number, default_limit);
            buffer.push(Some(PlayerDirectionUpdate {
                direction: Vec2::zero(),
                is_processed_client_input: None,
            }));
            buffer
        })
    }

    pub fn get_position_mut(
        &mut self,
        player_net_id: PlayerNetId,
        frame_number: FrameNumber,
        default_limit: u16,
    ) -> &mut Framebuffer<Option<Vec2>> {
        self.position.entry(player_net_id).or_insert_with(|| {
            let mut buffer = Framebuffer::new(frame_number, default_limit);
            buffer.push(Some(Vec2::zero()));
            buffer
        })
    }
}

#[derive(Clone, Copy)]
pub enum PlayerConnectionState {
    /// From the server's perspective it means that a StartGame message hasn't been sent yet.
    /// From the client's one - a client received a DeltaUpdate about a player that it doesn't know
    /// about.
    Connecting,
    Playing,
}

#[derive(Clone)]
pub struct Player {
    pub nickname: String,
    pub state: PlayerConnectionState,
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
