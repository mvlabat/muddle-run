#![feature(step_trait)]
#![feature(step_trait_ext)]
#![feature(trait_alias)]

use crate::{
    framebuffer::FrameNumber,
    game::{
        commands::{
            DespawnLevelObject, DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer,
        },
        level::LevelState,
        spawn::{spawn_level_objects, spawn_players},
    },
    net::network_setup,
    player::Player,
    registry::EntityRegistry,
};
use bevy::{core::FixedTimestep, prelude::*};
use messages::{EntityNetId, PlayerNetId};
use std::collections::HashMap;

pub mod framebuffer;
pub mod game;
pub mod looped_counter;
pub mod messages;
pub mod net;
pub mod player;
pub mod registry;

// Constants.
pub mod stage {
    pub const GAME: &str = "mr_shared_game";
    pub const POST_GAME: &str = "mr_shared_post_game";
}
pub const PLANE_SIZE: f32 = 10.0;

#[derive(Default)]
pub struct MuddleSharedPlugin;

impl Plugin for MuddleSharedPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        builder.add_stage_before(
            bevy::app::stage::UPDATE,
            stage::GAME,
            SystemStage::parallel().with_run_criteria(FixedTimestep::steps_per_second(60.0)),
        );
        builder.add_stage_after(
            stage::GAME,
            stage::POST_GAME,
            SystemStage::parallel().with_run_criteria(FixedTimestep::steps_per_second(60.0)),
        );

        builder.add_startup_system(network_setup.system());
        builder.add_system_to_stage(stage::GAME, spawn_players.system());
        builder.add_system_to_stage(stage::GAME, spawn_level_objects.system());
        builder.add_system_to_stage(stage::POST_GAME, tick.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(GameTime::default);
        resources.get_or_insert_with(LevelState::default);
        resources.get_or_insert_with(GameCommands::<SpawnPlayer>::default);
        resources.get_or_insert_with(GameCommands::<DespawnPlayer>::default);
        resources.get_or_insert_with(GameCommands::<SpawnLevelObject>::default);
        resources.get_or_insert_with(GameCommands::<DespawnLevelObject>::default);
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

pub fn tick(mut time: ResMut<GameTime>) {
    time.game_frame += FrameNumber::new(1);
}
