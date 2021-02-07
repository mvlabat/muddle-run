#![feature(step_trait)]
#![feature(step_trait_ext)]
#![feature(trait_alias)]
#![feature(generic_associated_types)]

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
use derivative::Derivative;
use std::marker::PhantomData;

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

unsafe impl<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner> Sync
    for MuddleSharedPlugin<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner>
{
}

unsafe impl<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner> Send
    for MuddleSharedPlugin<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner>
{
}

impl<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner> Plugin
    for MuddleSharedPlugin<PlayerSpawnerDeps, PlayerSpawner, PlaneSpawnerDeps, PlaneSpawner>
where
    PlayerSpawnerDeps: SystemParam + 'static,
    PlayerSpawner: Spawner<Dependencies = PlayerSpawnerDeps, Input = ()> + 'static,
    for<'a> <PlayerSpawnerDeps as SystemParam>::Fetch:
        FetchSystemParam<'a, Item = PlayerSpawnerDeps>,
    PlaneSpawnerDeps: SystemParam + 'static,
    PlaneSpawner: Spawner<Dependencies = PlaneSpawnerDeps, Input = PlaneDesc> + 'static,
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
