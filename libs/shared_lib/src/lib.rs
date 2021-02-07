#![feature(step_trait)]
#![feature(step_trait_ext)]
#![feature(trait_alias)]

use crate::{
    framebuffer::FrameNumber,
    game::{
        level_objects::PlaneDesc,
        spawn::{spawn_level_objects, spawn_player, EmptySpawner, Spawner},
    },
    net::PlayerNetId,
    registry::{EntityRegistry, Registry},
};
use bevy::{
    core::FixedTimestep,
    ecs::{FetchSystemParam, FuncSystem, SystemParam},
    prelude::*,
};
use std::marker::PhantomData;
use derivative::Derivative;

pub mod framebuffer;
pub mod game;
pub mod looped_counter;
pub mod net;
pub mod registry;

// Constants.
pub mod stage {
    pub const GAME: &str = "mr_shared_game";
}
const PLANE_SIZE: f32 = 10.0;

#[derive(Derivative)]
#[derivative(Default)]
pub struct MuddleSharedPlugin<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner> {
    _player_spawner_deps: PhantomData<PlayerSpawnerDeps>,
    _player_spawner: PhantomData<PlayerSpawner>,
    _plane_spawner_deps: PhantomData<PlaneSpawnerDeps>,
    _plane_spawner: PhantomData<PlaneSpawner>,
}

impl<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner> Plugin
    for MuddleSharedPlugin<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner>
where
    PlayerSpawnerDeps: SystemParam + Send + Sync + 'static,
    for<'a> PlayerSpawner:
        Spawner<'a, Dependencies = PlayerSpawnerDeps, Input = ()> + Send + Sync + 'static,
    for<'a> <PlayerSpawnerDeps as SystemParam>::Fetch:
        FetchSystemParam<'a, Item = PlayerSpawnerDeps>,
    PlaneSpawnerDeps: SystemParam + Send + Sync + 'static,
    for<'a> PlaneSpawner:
        Spawner<'a, Dependencies = PlaneSpawnerDeps, Input = PlaneDesc> + Send + Sync + 'static,
    for<'a> <PlaneSpawnerDeps as SystemParam>::Fetch: FetchSystemParam<'a, Item = PlaneSpawnerDeps>,
{
    fn build(&self, builder: &mut AppBuilder) {
        builder.add_stage_before(
            bevy::app::stage::UPDATE,
            stage::GAME,
            SystemStage::parallel().with_run_criteria(FixedTimestep::steps_per_second(30.0)),
        );

        builder.add_system_to_stage(
            stage::GAME,
            spawn_player::<PlayerSpawnerDeps, PlayerSpawner>.system(),
        );
        builder.add_system_to_stage(
            stage::GAME,
            spawn_level_objects::<PlaneSpawnerDeps, PlaneSpawner>.system(),
        );
    }
}

pub struct GameTime {
    /// Simulation frame.
    pub game_frame: FrameNumber,
}
