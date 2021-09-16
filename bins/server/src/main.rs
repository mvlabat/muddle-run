use bevy::app::App;
use mr_server_lib::MuddleServerPlugin;

fn main() {
    env_logger::init();

    // We want to exit the process on any panic (in any thread), so this is why the custom hook.
    let orig_hook = std::panic::take_hook();

    // And when I declared to my design
    // Like Frankenstein's monster
    // "I am your father, I am your god
    // And you the magic that I conjure"
    // — Stu Mackenzie
    std::panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);

        // TODO: track https://github.com/dimforge/rapier/issues/223 and fix `calculate_collider_shape` to avoid unwinding panics.
        if let Some(panic_info) = panic_info.location() {
            if panic_info.file().contains("parry") {
                return;
            }
        }

        // A kludge to let sentry send events first and then shutdown.
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::new(1, 0));
            std::process::exit(1);
        });
    }));

    let _guard = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        ..Default::default()
    });

    App::build().add_plugin(MuddleServerPlugin).run();
}
