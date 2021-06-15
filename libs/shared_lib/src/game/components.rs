use crate::{
    framebuffer::{FrameNumber, Framebuffer},
    COMPONENT_FRAMEBUFFER_LIMIT,
};
use bevy::math::Vec2;
use std::collections::VecDeque;

// NOTE: After adding components for new archetypes, make sure that related entities are cleaned up
// in the `restart_game` system.

pub struct PlayerTag;

pub struct LevelObjectTag;

pub struct LevelObjectLabel(pub String);

/// Represents Player's input (not an actual direction of entity's movement).
pub struct PlayerDirection {
    /// `None` indicates a missing network input.
    pub buffer: Framebuffer<Option<Vec2>>,
}

impl PlayerDirection {
    pub fn new(initial_value: Vec2, buffer_start_frame: FrameNumber, frames_to_fill: u16) -> Self {
        let mut buffer = Framebuffer::new(buffer_start_frame, COMPONENT_FRAMEBUFFER_LIMIT);
        for _ in 0..frames_to_fill {
            buffer.push(Some(initial_value));
        }
        Self { buffer }
    }
}

/// Represents start positions before moving an entity.
pub struct Position {
    pub buffer: Framebuffer<Vec2>,
}

impl Position {
    pub fn new(initial_value: Vec2, buffer_start_frame: FrameNumber, frames_to_fill: u16) -> Self {
        let mut buffer = Framebuffer::new(buffer_start_frame, COMPONENT_FRAMEBUFFER_LIMIT);
        for _ in 0..frames_to_fill {
            buffer.push(initial_value);
        }
        Self { buffer }
    }
}

/// Is used only by the client, to lerp the position if an authoritative update arrives from the
/// server. Using this component only makes sense if it's movement is not deterministic (i.e. it
/// can be affected by collisions with other entities or is controlled by a player, etc).
pub struct PredictedPosition {
    pub value: Vec2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpawnCommand {
    Spawn,
    Despawn,
}

/// The purpose of this component is providing a frame number of when a component was spawned,
/// to be able to avoid processing an entity in case rewind game state during lag compensation.
#[derive(Clone, Debug)]
pub struct Spawned {
    /// We store an option since FrameNumber represents a wrapped counter (i.e. cycling counter).
    /// If a component gets old enough, we set the timestamp to `None`, as we become sure that
    /// we won't try to simulate an entity that wasn't spawned for a given `GameTime::sumilation_frame`.
    /// See `mark_mature_entities` system.
    commands: VecDeque<(SpawnCommand, FrameNumber)>,
}

impl Spawned {
    pub fn new(frame_spawned: FrameNumber) -> Self {
        let mut commands = VecDeque::new();
        commands.push_back((SpawnCommand::Spawn, frame_spawned));
        Self { commands }
    }

    pub fn is_spawned(&self, frame_number: FrameNumber) -> bool {
        let mut res = true;
        for (command, _) in self
            .commands
            .iter()
            .take_while(|(_, command_frame_number)| frame_number >= *command_frame_number)
        {
            res = match command {
                SpawnCommand::Spawn => true,
                SpawnCommand::Despawn => false,
            }
        }
        res
    }

    pub fn can_be_removed(&self, frame_number: FrameNumber) -> bool {
        if let Some((SpawnCommand::Despawn, command_frame_number)) = self.commands.back() {
            return frame_number
                >= *command_frame_number + FrameNumber::new(COMPONENT_FRAMEBUFFER_LIMIT);
        }
        false
    }

    pub fn push_command(&mut self, frame_number: FrameNumber, command: SpawnCommand) {
        let command_differs = self.commands.is_empty()
            || self
                .commands
                .back()
                .map_or(false, |(last_command, _)| *last_command != command);
        let command_is_new = self
            .commands
            .back()
            .map_or(true, |(_, last_command_frame_number)| {
                *last_command_frame_number < frame_number
            });
        if command_differs && command_is_new {
            self.commands.push_back((command, frame_number));
        }
    }

    pub fn pop_outdated_commands(&mut self, frame_number: FrameNumber) {
        while matches!(self.commands.front(), Some((_, command_frame_number)) if frame_number > *command_frame_number + FrameNumber::new(COMPONENT_FRAMEBUFFER_LIMIT * 2))
        {
            self.commands.pop_front();
        }
    }
}

/// Marks an entity to be simulated with using `SimulationTime::player_frame`.
pub struct PlayerFrameSimulated;
