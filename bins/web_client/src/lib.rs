use bevy::prelude::*;
use mr_client_lib::MuddleClientPlugin;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main() {
    App::new()
        .insert_resource(Msaa { samples: 4 })
        .add_plugins(bevy::DefaultPlugins)
        .add_plugin(MuddleClientPlugin)
        .add_system(resize_canvas.system())
        .run();
}

fn resize_canvas(mut windows: ResMut<Windows>) {
    let window = match windows.get_primary_mut() {
        Some(window) => window,
        None => return,
    };

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
        window.set_resolution(width, height);
    }
}
