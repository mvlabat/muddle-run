use crate::{
    game::{
        commands::{GameCommands, SpawnLevelObject, SpawnLevelObjectDesc, SpawnPlayer},
        level_objects::PlaneDesc,
    },
    net::{EntityNetId, PlayerNetId},
    registry::EntityRegistry,
};
use bevy::{ecs::SystemParam, prelude::*};
use std::marker::PhantomData;

pub trait Spawner<'a> {
    type Dependencies;
    type Input;

    fn spawn(
        commands: &mut Commands,
        _deps: &mut Self::Dependencies,
        _input: &Self::Input,
    ) -> Entity {
        commands.spawn(());
        commands.current_entity().unwrap()
    }
}

#[derive(Default)]
pub struct EmptySpawner<I = ()> {
    _input: PhantomData<I>,
}

impl<'a, I> Spawner<'a> for EmptySpawner<I> {
    type Dependencies = ();
    type Input = I;
}

pub fn spawn_player<'a, D: SystemParam, S: Spawner<'a, Dependencies = D, Input = ()>>(
    commands: &mut Commands,
    mut dependencies: D,
    mut spawn_player_commands: ResMut<GameCommands<SpawnPlayer>>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
) {
    for command in spawn_player_commands.drain() {
        let player_entity = S::spawn(commands, &mut dependencies, &());
        player_entities.register(command.net_id, player_entity);
    }
}

pub fn spawn_level_objects<
    'a,
    PlaneSpawnerDeps: SystemParam,
    PlaneSpawner: Spawner<'a, Dependencies = PlaneSpawnerDeps, Input = PlaneDesc>,
>(
    commands: &mut Commands,
    mut dependencies: PlaneSpawnerDeps,
    mut spawn_level_object_commands: ResMut<GameCommands<SpawnLevelObject>>,
    mut object_entities: ResMut<EntityRegistry<EntityNetId>>,
) {
    for command in spawn_level_object_commands.drain() {
        let object_entity = match command.desc {
            SpawnLevelObjectDesc::Plane(plane) => {
                PlaneSpawner::spawn(commands, &mut dependencies, &plane)
            }
        };
        object_entities.register(command.net_id, object_entity);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::spawn::{EmptySpawner, Spawner};
    use bevy::ecs::{Commands, EntityReserver, Resources, World};

    #[test]
    fn test_empty_spawner() {
        let mut world = World::default();
        let mut resources = Resources::default();
        let mut command_buffer = Commands::default();
        command_buffer.set_entity_reserver(world.get_entity_reserver());

        let entity_a = EmptySpawner::spawn(&mut command_buffer, &mut (), &());
        let entity_b = EmptySpawner::spawn(&mut command_buffer, &mut (), &());
        assert_ne!(entity_a, entity_b);
    }
}
