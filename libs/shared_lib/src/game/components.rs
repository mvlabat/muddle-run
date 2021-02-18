use crate::{
    framebuffer::{FrameNumber, Framebuffer},
    COMPONENT_FRAMEBUFFER_LIMIT,
};
use bevy::math::Vec2;

/// Represents Player's input (not an actual direction of entity's movement).
#[derive(Debug)]
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

#[derive(Debug)]
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
