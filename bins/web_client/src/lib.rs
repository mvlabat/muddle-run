use bevy::{
    input::{keyboard::KeyboardInput, mouse::MouseButtonInput},
    prelude::*,
};
use wasm_bindgen::{prelude::*, JsCast};

#[derive(Default)]
pub struct WindowInnerSize {
    width: usize,
    height: usize,
}

#[derive(Default)]
pub struct MousePosition(Vec2);

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
        .add_resource(Msaa { samples: 4 })
        .add_plugins(DefaultPlugins)
        .init_resource::<WindowInnerSize>()
        .add_system(resize_canvas.system())
        // Startup systems,
        .add_startup_system(basic_scene.system())
        // Track input events.
        .init_resource::<TrackInputState>()
        .add_system(track_input_events.system())
        .run();
}

fn basic_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // add entities to the world
    commands
        // plane
        .spawn(PbrComponents {
            mesh: meshes.add(Mesh::from(shape::Plane { size: 10.0 })),
            material: materials.add(Color::rgb(0.3, 0.5, 0.3).into()),
            ..Default::default()
        })
        // cube
        .spawn(PbrComponents {
            mesh: meshes.add(Mesh::from(shape::Cube { size: 1.0 })),
            material: materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
            transform: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            ..Default::default()
        })
        // light
        .spawn(LightComponents {
            transform: Transform::from_translation(Vec3::new(4.0, 8.0, 4.0)),
            ..Default::default()
        })
        // camera
        .spawn(Camera3dComponents {
            transform: Transform::from_translation(Vec3::new(-3.0, 5.0, 8.0))
                .looking_at(Vec3::default(), Vec3::unit_y()),
            ..Default::default()
        });
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

#[derive(Default)]
struct TrackInputState {
    keys: EventReader<KeyboardInput>,
    cursor: EventReader<CursorMoved>,
    mouse_button: EventReader<MouseButtonInput>,
}

fn track_input_events(
    mut state: ResMut<TrackInputState>,
    mut mouse_position: ResMut<MousePosition>,
    ev_keys: Res<Events<KeyboardInput>>,
    ev_cursor: Res<Events<CursorMoved>>,
    ev_mouse_button: Res<Events<MouseButtonInput>>,
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
    if let Some(ev) = state.cursor.latest(&ev_cursor) {
        mouse_position.0 = ev.position;
    }

    // Mouse buttons.
    for ev in state.mouse_button.iter(&ev_mouse_button) {
        if ev.state.is_pressed() {
            log::info!("Just pressed mouse button: {:?}", ev.button);
        } else {
            log::info!("Just released mouse button: {:?}", ev.button);
        }
    }
}
