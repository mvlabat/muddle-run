use bevy::{
    ecs::{component::Component, entity::Entity},
    math::Vec2,
};

#[derive(Component)]
pub struct CameraPivotTag;

#[derive(Component)]
pub struct CameraPivotDirection(pub Vec2);

#[derive(Component)]
pub struct LevelObjectControlPoint;

#[derive(Component)]
pub struct LevelObjectControlBorder;

#[derive(Component)]
pub struct LevelObjectControlPoints {
    pub points: Vec<Entity>,
}

#[derive(Component)]
pub struct LevelObjectControlBorders {
    pub lines: Vec<(usize, Entity)>,
}
