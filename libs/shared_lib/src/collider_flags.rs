use bevy_rapier2d::rapier::geometry::InteractionGroups;

#[rustfmt::skip]
pub mod groups {
    pub const PLAYER: u32           = 0b00000000000000000000000000000001;
    pub const PLAYER_SENSOR: u32    = 0b00000000000000000000000000000010;
    pub const LEVEL_OBJECT: u32     = 0b00000000000000000000000000000100;
}

pub fn player_interaction_groups() -> InteractionGroups {
    InteractionGroups::new(groups::PLAYER, groups::LEVEL_OBJECT)
}

pub fn player_sensor_interaction_groups() -> InteractionGroups {
    InteractionGroups::new(groups::PLAYER_SENSOR, groups::LEVEL_OBJECT)
}

pub fn level_object_interaction_groups() -> InteractionGroups {
    InteractionGroups::new(groups::LEVEL_OBJECT, groups::PLAYER | groups::PLAYER_SENSOR)
}
