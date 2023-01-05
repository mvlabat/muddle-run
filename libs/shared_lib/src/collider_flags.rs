use bevy_rapier2d::prelude::CollisionGroups;

pub mod groups {
    use bevy_rapier2d::geometry::Group;

    pub const PLAYER: Group = Group::GROUP_1;
    pub const PLAYER_SENSOR: Group = Group::GROUP_2;
    pub const LEVEL_OBJECT: Group = Group::GROUP_3;

    pub const SERVER_PLAYER: Group = Group::GROUP_4;
    pub const SERVER_PLAYER_SENSOR: Group = Group::GROUP_5;
    pub const SERVER_LEVEL_OBJECT: Group = Group::GROUP_6;
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
