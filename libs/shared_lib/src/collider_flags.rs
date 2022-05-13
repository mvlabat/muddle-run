use bevy_rapier2d::prelude::CollisionGroups;

#[rustfmt::skip]
pub mod groups {
    pub const PLAYER: u32                   = 0b00000000000000000000000000000001;
    pub const PLAYER_SENSOR: u32            = 0b00000000000000000000000000000010;
    pub const LEVEL_OBJECT: u32             = 0b00000000000000000000000000000100;

    pub const SERVER_PLAYER: u32            = 0b00000000000000000000000000001000;
    pub const SERVER_PLAYER_SENSOR: u32     = 0b00000000000000000000000000010000;
    pub const SERVER_LEVEL_OBJECT: u32      = 0b00000000000000000000000000100000;
}

pub fn player_collision_groups(server_simulated: bool) -> CollisionGroups {
    if server_simulated {
        CollisionGroups::new(groups::SERVER_PLAYER, groups::SERVER_LEVEL_OBJECT)
    } else {
        CollisionGroups::new(groups::PLAYER, groups::LEVEL_OBJECT)
    }
}

pub fn player_sensor_collision_groups(server_simulated: bool) -> CollisionGroups {
    if server_simulated {
        CollisionGroups::new(groups::SERVER_PLAYER_SENSOR, groups::SERVER_LEVEL_OBJECT)
    } else {
        CollisionGroups::new(groups::PLAYER_SENSOR, groups::LEVEL_OBJECT)
    }
}

pub fn level_object_collision_groups(server_simulated: bool) -> CollisionGroups {
    if server_simulated {
        CollisionGroups::new(
            groups::SERVER_LEVEL_OBJECT,
            groups::SERVER_PLAYER | groups::SERVER_PLAYER_SENSOR,
        )
    } else {
        CollisionGroups::new(groups::LEVEL_OBJECT, groups::PLAYER | groups::PLAYER_SENSOR)
    }
}
