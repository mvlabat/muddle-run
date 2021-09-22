use crate::{
    framebuffer::{FrameNumber, Framebuffer},
    messages::{PlayerNetId, RespawnPlayerReason},
    registry::EntityRegistry,
};
use bevy::{
    ecs::system::{ResMut, SystemParam},
    log,
    math::Vec2,
    utils::HashMap,
};
use serde::{Deserialize, Serialize};

#[derive(SystemParam)]
pub struct PlayerSystemParamsMut<'a> {
    pub players: ResMut<'a, HashMap<PlayerNetId, Player>>,
    pub player_registry: ResMut<'a, EntityRegistry<PlayerNetId>>,
}

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
            log::debug!(
                "Create a new direction buffer (client: {:?}, frame: {})",
                player_net_id,
                frame_number
            );
            let mut buffer = Framebuffer::new(frame_number, default_limit);
            buffer.push(Some(PlayerDirectionUpdate {
                direction: Vec2::ZERO,
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
            log::debug!(
                "Create a new position buffer (client: {:?}, frame: {})",
                player_net_id,
                frame_number
            );
            let mut buffer = Framebuffer::new(frame_number, default_limit);
            buffer.push(Some(Vec2::ZERO));
            buffer
        })
    }
}

#[derive(Clone, Debug)]
pub struct Player {
    pub nickname: String,
    pub role: PlayerRole,
    pub respawning_at: Option<(FrameNumber, RespawnPlayerReason)>,
    pub is_connected: bool,
    pub finishes: u32,
    pub deaths: u32,
}

impl Player {
    pub fn new(role: PlayerRole) -> Player {
        Player {
            nickname: "?".to_owned(),
            role,
            respawning_at: None,
            is_connected: true,
            finishes: 0,
            deaths: 0,
        }
    }

    pub fn new_with_nickname(role: PlayerRole, nickname: String) -> Player {
        Player {
            nickname,
            role,
            respawning_at: None,
            is_connected: true,
            finishes: 0,
            deaths: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum PlayerRole {
    Runner,
    Builder,
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
