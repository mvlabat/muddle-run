use bevy::{ecs::entity::Entity, math::Vec2};

pub struct CameraPivotTag;

pub struct CameraPivotDirection(pub Vec2);

pub struct LevelObjectControlPoint;

pub struct LevelObjectControlBorder;

pub struct LevelObjectControlPoints {
    pub points: Vec<Entity>,
}

pub struct LevelObjectControlBorders {
    pub lines: Vec<(usize, Entity)>,
}
