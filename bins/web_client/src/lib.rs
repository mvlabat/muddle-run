use bevy::{
    input::{
        keyboard::KeyboardInput,
        mouse::{MouseButtonInput, MouseMotion, MouseWheel},
    },
    prelude::*,
};
use wasm_bindgen::prelude::*;
// use wasm_bindgen::JsCast;

#[wasm_bindgen(start)]
pub fn main() {
    // std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    // console_log::init_with_level(log::Level::Debug).expect("cannot initialize console_log");

    App::build()
        .add_resource(WindowDescriptor {
            width: 1366,
            height: 768,
            ..Default::default()
        })
        .add_plugins(DefaultPlugins)
        // One time greet
        .add_startup_system(hello_wasm_system.system())
        // Track ticks (sanity check, whether game loop is running)
        .add_system(counter.system())
        // .add_system(resize_canvas.system())
        // Track input events
        .init_resource::<TrackInputState>()
        .add_system(track_input_events.system())
        .run();
}

fn hello_wasm_system() {
    log::info!("hello wasm");
}

fn counter(mut state: Local<CounterState>, time: Res<Time>) {
    if state.count % 60 == 0 {
        log::info!(
            "tick {} @ {:?} [Δ{}]",
            state.count,
            time.time_since_startup(),
            time.delta_seconds
        );
    }
    state.count += 1;
}

// fn resize_canvas() {
//     let window = web_sys::window().expect("no global `window` exists");
//     let document = window.document().expect("should have a document on window");
//     let body = document.body().expect("document should have a body");
//
//     let canvas = document
//         .get_elements_by_tag_name("canvas")
//         .item(0)
//         .expect("body should have a canvas")
//         .dyn_into::<web_sys::HtmlCanvasElement>()
//         .expect("expected to be a canvas");
//
//     canvas
//         .style()
//         .set_property("width", &body.client_width().to_string())
//         .expect("expected to set canvas width");
//     canvas
//         .style()
//         .set_property("height", &body.client_height().to_string())
//         .expect("expected to set canvas height");
//     canvas
//         .set_attribute("width", &body.client_width().to_string())
//         .expect("expected to set canvas width");
//     canvas
//         .set_attribute("height", &body.client_height().to_string())
//         .expect("expected to set canvas height");
// }

#[derive(Default)]
struct CounterState {
    count: u32,
}

#[derive(Default)]
struct TrackInputState {
    keys: EventReader<KeyboardInput>,
    cursor: EventReader<CursorMoved>,
    motion: EventReader<MouseMotion>,
    mousebtn: EventReader<MouseButtonInput>,
    scroll: EventReader<MouseWheel>,
}

fn track_input_events(
    mut state: ResMut<TrackInputState>,
    ev_keys: Res<Events<KeyboardInput>>,
    ev_cursor: Res<Events<CursorMoved>>,
    ev_motion: Res<Events<MouseMotion>>,
    ev_mousebtn: Res<Events<MouseButtonInput>>,
    ev_scroll: Res<Events<MouseWheel>>,
) {
    // Keyboard input.
    for ev in state.keys.iter(&ev_keys) {
        if ev.state.is_pressed() {
            log::info!("Just pressed key: {:?}", ev.key_code);
        } else {
            log::info!("Just released key: {:?}", ev.key_code);
        }
    }

    // Absolute cursor position (in window coordinates).
    for ev in state.cursor.iter(&ev_cursor) {
        log::info!("Cursor at: {}", ev.position);
    }

    // Relative mouse motion.
    for ev in state.motion.iter(&ev_motion) {
        log::info!("Mouse moved {} pixels", ev.delta);
    }

    // Mouse buttons.
    for ev in state.mousebtn.iter(&ev_mousebtn) {
        if ev.state.is_pressed() {
            log::info!("Just pressed mouse button: {:?}", ev.button);
        } else {
            log::info!("Just released mouse button: {:?}", ev.button);
        }
    }

    // Scrolling.
    for ev in state.scroll.iter(&ev_scroll) {
        log::info!(
            "Scrolled vertically by {} and horizontally by {}.",
            ev.y,
            ev.x
        );
    }
}
