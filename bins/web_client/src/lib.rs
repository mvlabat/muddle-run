use bevy::prelude::*;
use mr_client_shared::{MuddlePlugin, WindowInnerSize};
use wasm_bindgen::{prelude::*, JsCast};

#[wasm_bindgen(start)]
pub fn main() {
    App::build()
        .add_plugins(bevy_webgl2::DefaultPlugins)
        .add_plugin(MuddlePlugin)
        .add_system(resize_canvas)
        .run();
}

fn resize_canvas(mut window_inner_size: ResMut<WindowInnerSize>) {
    let window = web_sys::window().expect("no global `window` exists");

    let width = window
        .inner_width()
        .expect("window should have size")
        .as_f64()
        .unwrap() as usize;
    let height = window
        .inner_height()
        .expect("window should have size")
        .as_f64()
        .unwrap() as usize;

    if window_inner_size.width != width || window_inner_size.height != height {
        window_inner_size.width = width;
        window_inner_size.height = height;

        let document = window.document().expect("should have a document on window");
        let canvas = document
            .get_elements_by_tag_name("canvas")
            .item(0)
            .expect("body should have a canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("expected to be a canvas");

        canvas
            .style()
            .set_css_text(&format!("width: {}px; height: {}px;", width, height));
        canvas
            .set_attribute("width", &width.to_string())
            .expect("expected to set canvas width");
        canvas
            .set_attribute("height", &height.to_string())
            .expect("expected to set canvas width");
    }
}
