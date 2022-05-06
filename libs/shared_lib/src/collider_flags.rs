use bevy_rapier2d::prelude::CollisionGroups;

#[rustfmt::skip]
pub mod groups {
    pub const PLAYER: u32           = 0b00000000000000000000000000000001;
    pub const PLAYER_SENSOR: u32    = 0b00000000000000000000000000000010;
    pub const LEVEL_OBJECT: u32     = 0b00000000000000000000000000000100;
}

pub fn player_collision_groups() -> CollisionGroups {
    CollisionGroups::new(groups::PLAYER, groups::LEVEL_OBJECT)
}

pub fn player_sensor_collision_groups() -> CollisionGroups {
    CollisionGroups::new(groups::PLAYER_SENSOR, groups::LEVEL_OBJECT)
}

pub fn level_object_collision_groups() -> CollisionGroups {
    CollisionGroups::new(groups::LEVEL_OBJECT, groups::PLAYER | groups::PLAYER_SENSOR)
}
