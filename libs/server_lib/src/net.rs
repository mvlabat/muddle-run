use crate::player_updates::DeferredUpdates;
use bevy::{ecs::ResMut, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource, NetworkingPlugin, Packet};
use mr_shared_lib::net::{deserialize, ClientMessage, PlayerNetId, PlayerInput};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};
use mr_shared_lib::registry::Registry;

const SERVER_PORT: u16 = 3455;

pub fn startup(mut net: ResMut<NetworkResource>) {
    let socket_address: SocketAddr = SocketAddr::new(IpAddr::from([0, 0, 0, 0]), SERVER_PORT);
    log::info!("Starting the server");
    net.listen(socket_address);
}

#[derive(Default)]
pub struct NetworkReader {
    network_events: EventReader<NetworkEvent>,
}

pub type PlayerConnections = Registry<PlayerNetId, u32>;

pub fn process_network_events(
    mut net: ResMut<NetworkResource>,
    time: Res<Time>,
    mut state: ResMut<NetworkReader>,
    network_events: Res<Events<NetworkEvent>>,
    mut player_connections: ResMut<PlayerConnections>,
    mut deferred_player_updates: ResMut<DeferredUpdates<PlayerInput>>,
) {
    for event in state.network_events.iter(&network_events) {
        match event {
            NetworkEvent::Packet(handle, packet) => {
                let message: ClientMessage = match deserialize(packet) {
                    Ok(message) => message,
                    Err(err) => {
                        log::warn!("Failed to deserialize message (from [{}])", handle);
                        continue;
                    }
                };

                log::trace!("Got packet on [{}]: {:?}", handle, message);

                let player_net_id = match player_connections.get_id(*handle) {
                    Some(id) => id,
                    None => {
                        log::error!("A player for handle {} is not registered", handle);
                        continue;
                    }
                };

                match message {
                    ClientMessage::PlayerInput(update) => {
                        deferred_player_updates.push(player_net_id, update)
                    }
                }
            }
            NetworkEvent::Connected(handle) => {
                log::info!("New connection: {}", handle);
                let player_net_id = player_connections.register(*handle);
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
                player_connections.remove_by_value(*handle);
            }
        }
    }
}

fn send_network_updates(mut net: ResMut<NetworkResource>) {}
