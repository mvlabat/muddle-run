use crate::{
    framebuffer::{FrameNumber, Framebuffer},
    game::level::CollisionLogic,
    COMPONENT_FRAMEBUFFER_LIMIT,
};
use bevy::{
    ecs::{bundle::Bundle, component::Component, entity::Entity},
    math::Vec2,
    utils::HashSet,
};
use bevy_rapier2d::{
    dynamics::{LockedAxes, RigidBody},
    geometry::{Collider, CollisionGroups, Sensor},
};
use std::collections::VecDeque;

// NOTE: After adding components for new archetypes, make sure that related entities are cleaned up
// in the `restart_game` system.

#[derive(Bundle)]
pub struct PhysicsBundle {
    pub rigid_body: RigidBody,
    pub collider: Collider,
    pub sensor: Sensor,
    pub collision_groups: CollisionGroups,
    pub locked_axes: LockedAxes,
}

#[derive(Component)]
pub struct PlayerTag;

#[derive(Component, Clone)]
pub struct PlayerSensor(pub Entity);

#[derive(Component, Debug)]
pub struct PlayerSensors {
    pub main: PlayerSensorState,
    pub sensors: Vec<(Entity, PlayerSensorState)>,
}

impl PlayerSensors {
    pub fn player_is_dead(&self) -> bool {
        let sensors_contact_death_or_nothing = self
            .sensors
            .iter()
            .any(|(_, sensor)| sensor.contacting.is_empty() || sensor.has(CollisionLogic::Death));
        self.main.has(CollisionLogic::Death)
            || self.main.contacting.is_empty()
            || sensors_contact_death_or_nothing
    }

    pub fn player_has_finished(&self) -> bool {
        let sensors_contact_finish = self
            .sensors
            .iter()
            .any(|(_, sensor)| sensor.has(CollisionLogic::Finish));
        self.main.has(CollisionLogic::Finish) || sensors_contact_finish
    }
}

#[derive(Default, Debug)]
pub struct PlayerSensorState {
    /// Includes both contact and intersection events.
    pub contacting: Vec<(Entity, CollisionLogic)>,
}

impl PlayerSensorState {
    pub fn has(&self, collision_logic: CollisionLogic) -> bool {
        self.contacting
            .iter()
            .any(|(_, logic)| *logic == collision_logic)
    }
}

#[derive(Component, Default)]
pub struct LevelObjectTag;

#[derive(Component, Default)]
pub struct LevelObjectLabel(pub String);

/// Entity having this component represents a level object at its initial position.
/// This component points to a parent, so an entity having this component is a child.
#[derive(Component)]
pub struct LevelObjectStaticGhostParent(pub Entity);

#[derive(Component)]
pub struct LevelObjectStaticGhostChild(pub Entity);

/// Entity having this component represents a level object simulated at server time.
/// This component points to a parent, so an entity having this component is a child.
#[derive(Component, Debug)]
pub struct LevelObjectServerGhostParent(pub Entity);

/// Points to an entity that is a ghost of a level object simulated at server time.
#[derive(Component, Debug)]
pub struct LevelObjectServerGhostChild(pub Entity);

/// Represents Player's input (not an actual direction of entity's movement).
#[derive(Component, Debug)]
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
#[derive(Component, Debug)]
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

    pub fn take(&mut self) -> Self {
        Position {
            buffer: self.buffer.take(),
        }
    }
}

/// Is used only by the client, to lerp the position if an authoritative update arrives from the
/// server. Using this component only makes sense if it's movement is not deterministic (i.e. it
/// can be affected by collisions with other entities or is controlled by a player, etc).
#[derive(Component, Debug)]
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
#[derive(Component, Clone, Debug, Default)]
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
        let mut command_index: Option<usize> = None;
        for (i, (command, _)) in self
            .commands
            .iter()
            .enumerate()
            .take_while(|(_, (_, command_frame_number))| frame_number >= *command_frame_number)
        {
            command_index = Some(i);
            res = match command {
                SpawnCommand::Spawn => true,
                SpawnCommand::Despawn => false,
            }
        }
        // If there is the next command, and it's a Spawn command, the entity isn't spawned yet.
        if let Some((SpawnCommand::Spawn, _)) =
            self.commands.get(command_index.map_or(0, |i| i + 1))
        {
            return false;
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
#[derive(Component, Debug)]
pub struct PlayerFrameSimulated;

#[derive(Component, Clone)]
pub struct LevelObjectMovement {
    /// May point to the future if it's the start of the game and we have non-zero start offset.
    pub frame_started: FrameNumber,
    /// Represents a vector from the attached route point to the initial object position.
    /// Matters only for radial movement.
    pub init_vec: Vec2,
    /// Zero period means the object is always staying at the initial position or doesn't rotate.
    pub period: FrameNumber,
    /// How much route progress each point corresponds to. The final one must always equal `1.0`.
    /// For the radial movement type it contains only 1 element: the attached object (the center).
    pub points_progress: Vec<LevelObjectMovementPoint>,
    pub movement_type: LevelObjectMovementType,
}

/// A marker component to tag an entity that is excluded from physics simulations.
#[derive(Component, Clone, Copy)]
pub struct LockPhysics(pub bool);

#[derive(Clone)]
pub struct LevelObjectMovementPoint {
    pub progress: f32,
    pub position: Vec2,
    pub entity: Entity,
}

/// Maps to `ObjectRouteDesc`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LevelObjectMovementType {
    /// Corresponds to either `ObjectRouteDesc::ForwardCycle` or `ObjectRouteDesc::ForwardBackwardsCycle`.
    Linear,
    /// If `LevelObjectMovement::period` equals 0, this corresponds to `ObjectRouteDesc::Attached`.
    Radial,
}

impl LevelObjectMovement {
    pub fn total_progress(&self, frame_number: FrameNumber) -> f32 {
        if self.period == FrameNumber::new(0) {
            return 0.0;
        }

        let frame_number = if self.frame_started > frame_number {
            frame_number + self.period - self.frame_started
        } else {
            frame_number - self.frame_started
        };
        assert!(frame_number < self.period);

        frame_number.value() as f32 / self.period.value() as f32
    }

    pub fn dependencies(&self) -> HashSet<Entity> {
        self.points_progress
            .iter()
            .map(|point| point.entity)
            .collect()
    }

    pub fn current_position(&self, frame_number: FrameNumber) -> Vec2 {
        match self.movement_type {
            LevelObjectMovementType::Linear => self.current_position_linear(frame_number),
            LevelObjectMovementType::Radial => self.current_position_radial(frame_number),
        }
    }

    fn current_position_linear(&self, frame_number: FrameNumber) -> Vec2 {
        if self.points_progress.is_empty() {
            return self.init_vec;
        }
        if self.points_progress.len() == 1 {
            return self.points_progress[0].position;
        }

        let progress = self.total_progress(frame_number);
        let mut current_point_progress = 0.0;
        let mut next_point_progress = 0.0;
        let mut next_point_index = 0usize;
        for (i, point) in self.points_progress.iter().enumerate() {
            if point.progress > progress {
                next_point_index = i;
                next_point_progress = point.progress;
                break;
            }
            if point.progress - current_point_progress > f32::EPSILON {
                current_point_progress = point.progress;
            }
        }

        #[allow(clippy::float_cmp)]
        if (progress - 1.0).abs() > f32::EPSILON {
            assert_ne!(current_point_progress, next_point_progress);
        } else {
            return self.points_progress[next_point_index].position;
        }
        let progress_between_points =
            1.0 - (next_point_progress - progress) / (next_point_progress - current_point_progress);

        let current_point_position = self.points_progress[next_point_index - 1].position;
        let next_point_position = self.points_progress[next_point_index].position;
        current_point_position.lerp(next_point_position, progress_between_points)
    }

    fn current_position_radial(&self, frame_number: FrameNumber) -> Vec2 {
        let center = self
            .points_progress
            .get(0)
            .expect("Expected the first element of `points_progress` (the circle center) to exist")
            .position;
        let radius = rotate(
            self.init_vec,
            self.total_progress(frame_number) * std::f32::consts::PI * 2.0,
        );
        center + radius
    }
}

// TODO: move to a separate math module.
pub fn rotate(v: Vec2, radians: f32) -> Vec2 {
    Vec2::new(
        v.x * radians.cos() - v.y * radians.sin(),
        v.x * radians.sin() + v.y * radians.cos(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Vec2;

    fn assert_eq_vec(v1: Vec2, v2: Vec2) {
        let v = (v1 - v2).abs();
        assert!(v.x < f32::EPSILON && v.y < f32::EPSILON);
    }

    #[test]
    fn test_total_progress() {
        let level_object_movement = LevelObjectMovement {
            frame_started: FrameNumber::new(5),
            init_vec: Vec2::ZERO,
            period: FrameNumber::new(10),
            points_progress: Vec::new(),
            movement_type: LevelObjectMovementType::Linear,
        };
        assert!(
            (level_object_movement.total_progress(FrameNumber::new(u16::MAX - 4)) - 0.0).abs()
                < f32::EPSILON
        );
        assert!(
            (level_object_movement.total_progress(FrameNumber::new(5)) - 0.0).abs() < f32::EPSILON
        );
        assert!(
            (level_object_movement.total_progress(FrameNumber::new(0)) - 0.5).abs() < f32::EPSILON
        );
        assert!(
            (level_object_movement.total_progress(FrameNumber::new(10)) - 0.5).abs() < f32::EPSILON
        );
    }

    #[test]
    fn test_rotate() {
        assert_eq_vec(
            rotate(Vec2::new(1.0, 0.0), std::f32::consts::PI / 2.0),
            Vec2::new(0.0, 1.0),
        );
        assert_eq_vec(
            rotate(Vec2::new(0.0, 1.0), std::f32::consts::PI / 2.0),
            Vec2::new(-1.0, 0.0),
        );
        assert_eq_vec(
            rotate(Vec2::new(1.0, 0.0), std::f32::consts::PI),
            Vec2::new(-1.0, 0.0),
        );
    }
}
