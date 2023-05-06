#![allow(clippy::unused_unit)]

use bevy::{prelude::*, window::PrimaryWindow};
use mr_client_lib::{MuddleClientConfig, MuddleClientPlugin, DEFAULT_SERVER_PORT};
use mr_utils_lib::try_parse_from_env;
use std::net::SocketAddr;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main() {
    App::new()
        .insert_resource(MuddleClientConfig {
            persistence_url: try_parse_from_env!("MUDDLE_PUBLIC_PERSISTENCE_URL"),
            google_client_id: try_parse_from_env!("MUDDLE_GOOGLE_CLIENT_ID"),
            google_client_secret: try_parse_from_env!("MUDDLE_GOOGLE_CLIENT_SECRET"),
            auth0_client_id: try_parse_from_env!("MUDDLE_AUTH0_CLIENT_ID"),
            matchmaker_url: try_parse_from_env!("MUDDLE_MATCHMAKER_URL"),
            server_addr: server_addr(),
        })
        .insert_resource(Msaa::Sample4)
        .add_plugins(bevy::DefaultPlugins)
        .add_plugin(MuddleClientPlugin)
        .add_system(resize_canvas)
        .run();
}

fn resize_canvas(mut windows: Query<&'static mut Window, With<PrimaryWindow>>) {
    let Ok(mut window) = windows.get_single_mut() else { return };

    let js_window = web_sys::window().expect("no global `window` exists");

    let width = js_window
        .inner_width()
        .expect("window should have size")
        .as_f64()
        .unwrap() as f32;
    let height = js_window
        .inner_height()
        .expect("window should have size")
        .as_f64()
        .unwrap() as f32;

    #[allow(clippy::float_cmp)]
    if window.width() != width || window.height() != height {
        window.resolution.set(width, height);
    }
}

fn server_addr() -> Option<SocketAddr> {
    let port: u16 = try_parse_from_env!("MUDDLE_SERVER_PORT").unwrap_or(DEFAULT_SERVER_PORT);
    try_parse_from_env!("MUDDLE_SERVER_IP_ADDR").map(|ip_addr| SocketAddr::new(ip_addr, port))
}
