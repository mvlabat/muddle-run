use bevy::app::App;
use mr_server_lib::MuddleServerPlugin;

fn main() {
    env_logger::init();

    std::panic::set_hook(Box::new(|_| {
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
