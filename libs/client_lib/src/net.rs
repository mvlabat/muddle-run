use std::net::SocketAddr;

use bevy::{log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};

use mr_shared_lib::net::{deserialize, serialize, ClientMessage, PlayerInput, ServerMessage};
use mr_shared_lib::framebuffer::FrameNumber;

const SERVER_PORT: u16 = 3455;

pub fn initiate_connection(mut net: ResMut<NetworkResource>) {
    if net.connections.is_empty() {
        // TODO: pass from command-line.
        let server_ip_addr = bevy_networking_turbulence::find_my_ip_address()
            .expect("cannot find current ip address");
        let socket_address: SocketAddr = SocketAddr::new(server_ip_addr, SERVER_PORT);
        log::info!("Starting the client");
        net.connect(socket_address);
    }
}

#[derive(Default)]
pub struct NetworkReader {
    network_events: EventReader<NetworkEvent>,
}

pub fn process_network_events(
    _net: Res<NetworkResource>,
    mut state: Local<NetworkReader>,
    network_events: Res<Events<NetworkEvent>>,
) {
    for event in state.network_events.iter(&network_events) {
        match event {
            NetworkEvent::Packet(handle, packet) => {
                let message: ServerMessage = match deserialize(packet) {
                    Ok(message) => message,
                    Err(err) => {
                        log::warn!(
                            "Failed to deserialize message (from [{}]): {:?}",
                            handle,
                            err
                        );
                        continue;
                    }
                };

                log::info!("Got packet on [{}]: {:?}", handle, message);
            }
            NetworkEvent::Connected(handle) => {
                log::info!("Connected: {}", handle);
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
            }
        }
    }
}

pub fn send_network_updates(mut net: ResMut<NetworkResource>) {
    let (_handle, connection) = match net.connections.iter_mut().next() {
        Some(connection) => connection,
        None => return,
    };
    if let Err(err) = connection.send(
        serialize(&ClientMessage::PlayerInput(PlayerInput {
            frame_number: FrameNumber::new(0),
            direction: Vec2::new(0.0, 0.0),
        }))
        .unwrap()
        .into(),
    ) {
        log::error!(
            "Failed to send message to {:?}: {:?}",
            connection.remote_address(),
            err
        );
    }
}
