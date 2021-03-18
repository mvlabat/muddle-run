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
    pub fn new(initial_value: Vec2, buffer_start_frame: FrameNumber) -> Self {
        let mut buffer = Framebuffer::new(buffer_start_frame, COMPONENT_FRAMEBUFFER_LIMIT);
        buffer.push(Some(initial_value));
        Self { buffer }
    }
}

/// Represents start positions before moving an entity.
pub struct Position {
    pub buffer: Framebuffer<Vec2>,
}

impl Position {
    pub fn new(initial_value: Vec2, buffer_start_frame: FrameNumber) -> Self {
        let mut buffer = Framebuffer::new(buffer_start_frame, COMPONENT_FRAMEBUFFER_LIMIT);
        buffer.push(initial_value);
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
    fresh_timestamp: Option<FrameNumber>,
}

impl Spawned {
    pub fn new(frame_spawned: FrameNumber) -> Self {
        Self {
            fresh_timestamp: Some(frame_spawned),
        }
    }

    pub fn is_spawned(&self, frame_number: FrameNumber) -> bool {
        self.fresh_timestamp
            .map_or(true, |fresh_timestamp| fresh_timestamp <= frame_number)
    }

    pub fn mark_if_mature(&mut self, frame_number: FrameNumber) {
        if let Some(fresh_timestamp) = self.fresh_timestamp {
            if frame_number > fresh_timestamp + FrameNumber::new(COMPONENT_FRAMEBUFFER_LIMIT) {
                self.fresh_timestamp = None;
            }
        }
    }
}
