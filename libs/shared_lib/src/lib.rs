#![feature(step_trait)]
#![feature(step_trait_ext)]
#![feature(trait_alias)]

use crate::{
    framebuffer::FrameNumber,
    game::{
        commands::{GameCommands, SpawnLevelObject, SpawnPlayer},
        level::LevelState,
        spawn::{spawn_level_objects, spawn_player},
    },
    net::{EntityNetId, PlayerNetId},
    player::Player,
    registry::EntityRegistry,
};
use bevy::{core::FixedTimestep, prelude::*};
use std::collections::HashMap;

pub mod framebuffer;
pub mod game;
pub mod looped_counter;
pub mod net;
pub mod player;
pub mod registry;

// Constants.
pub mod stage {
    pub const GAME: &str = "mr_shared_game";
}
pub const PLANE_SIZE: f32 = 10.0;

#[derive(Default)]
pub struct MuddleSharedPlugin;

impl Plugin for MuddleSharedPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        builder.add_stage_before(
            bevy::app::stage::UPDATE,
            stage::GAME,
            SystemStage::parallel().with_run_criteria(FixedTimestep::steps_per_second(30.0)),
        );

        builder.add_system_to_stage(stage::GAME, spawn_player.system());
        builder.add_system_to_stage(stage::GAME, spawn_level_objects.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(GameTime::default);
        resources.get_or_insert_with(LevelState::default);
        resources.get_or_insert_with(GameCommands::<SpawnPlayer>::default);
        resources.get_or_insert_with(GameCommands::<SpawnLevelObject>::default);
        resources.get_or_insert_with(EntityRegistry::<PlayerNetId>::default);
        resources.get_or_insert_with(EntityRegistry::<EntityNetId>::default);
        resources.get_or_insert_with(HashMap::<PlayerNetId, Player>::default);
    }
}

#[derive(Default)]
pub struct GameTime {
    /// Simulation frame.
    pub game_frame: FrameNumber,
}
