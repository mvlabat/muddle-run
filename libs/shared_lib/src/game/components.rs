use crate::{
    framebuffer::{FrameNumber, Framebuffer},
    COMPONENT_FRAMEBUFFER_LIMIT,
};
use bevy::math::Vec2;

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

/// The purpose of this component is providing a frame number of when a component was spawned,
/// to be able to avoid processing an entity in case rewind game state during lag compensation.
pub struct Spawned {
    /// We store an option since FrameNumber represents a wrapped counter (i.e. cycling counter).
    /// If a component gets old enough, we set the timestamp to `None`, as we become sure that
    /// we won't try to simulate an entity that wasn't spawned for a given `GameTime::sumilation_frame`.
    /// See `mark_mature_entities` system.
    spawned_at: Option<FrameNumber>,
    despawned_at: Option<FrameNumber>,
    respawned_at: Option<FrameNumber>,
}

impl Spawned {
    pub fn new(frame_spawned: FrameNumber) -> Self {
        Self {
            spawned_at: Some(frame_spawned),
            despawned_at: None,
            respawned_at: None,
        }
    }

    pub fn is_spawned(&self, frame_number: FrameNumber) -> bool {
        let is_spawned = self
            .spawned_at
            .map_or(true, |spawned_at| spawned_at <= frame_number);
        is_spawned && !self.is_despawned(frame_number)
    }

    pub fn is_despawned(&self, frame_number: FrameNumber) -> bool {
        match (self.despawned_at, self.respawned_at) {
            (Some(despawned_at), None) => frame_number >= despawned_at,
            (Some(despawned_at), Some(respawned_at)) => {
                frame_number >= despawned_at && frame_number < respawned_at
            }
            (None, _) => false,
        }
    }

    pub fn mark_if_mature(&mut self, frame_number: FrameNumber) {
        if let Some(spawned_at) = self.spawned_at {
            if frame_number > spawned_at + FrameNumber::new(COMPONENT_FRAMEBUFFER_LIMIT) {
                self.spawned_at = None;
            }
        }
        let despawned_long_time_ago = self.despawned_at.map_or(false, |despawned_at| {
            frame_number > despawned_at + FrameNumber::new(COMPONENT_FRAMEBUFFER_LIMIT)
        });
        if !self.can_be_removed(frame_number) && despawned_long_time_ago {
            self.despawned_at = None;
            self.respawned_at = None;
        }
    }

    pub fn set_despawned_at(&mut self, frame_number: FrameNumber) {
        self.despawned_at = Some(frame_number);
    }

    pub fn set_respawned_at(&mut self, frame_number: FrameNumber) {
        self.respawned_at = Some(frame_number);
    }

    pub fn can_be_removed(&self, frame_number: FrameNumber) -> bool {
        let can_be_despawned = self.despawned_at.map_or(false, |despawned_at| {
            despawned_at + FrameNumber::new(COMPONENT_FRAMEBUFFER_LIMIT) >= frame_number
        });
        can_be_despawned && self.respawned_at.is_none()
    }
}

/// Marks an entity to be simulated with using `SimulationTime::player_frame`.
pub struct PlayerFrameSimulated;
