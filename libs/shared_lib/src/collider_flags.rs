use crate::collider_flags::groups::{LEVEL_OBJECT_GROUP, PLAYER_GROUP};
use bevy_rapier2d::rapier::geometry::InteractionGroups;

#[rustfmt::skip]
pub mod groups {
    pub const PLAYER_GROUP: u32         = 0b00000000000000000000000000000001;
    pub const LEVEL_OBJECT_GROUP: u32   = 0b00000000000000000000000000000010;
}

pub fn player_interaction_groups() -> InteractionGroups {
    InteractionGroups::new(PLAYER_GROUP, LEVEL_OBJECT_GROUP)
}

pub fn level_object_interaction_groups() -> InteractionGroups {
    InteractionGroups::new(LEVEL_OBJECT_GROUP, PLAYER_GROUP)
}
