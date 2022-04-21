#[cfg(feature = "bevy_logging")]
use bevy::log;

pub fn load_env() {
    // Sync this with `build_dotenv`.
    let muddle_env = std::env::var("MUDDLE_ENV").unwrap_or_else(|_| {
        match std::env::var("PROFILE").as_deref() {
            Ok("release") => "production",
            Ok("debug") => "development",
            _ => "development",
        }
        .to_owned()
    });
    let Some(package_name) = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|path| path.to_string_lossy().to_string())
        })
    else {
        return;
    };
    let filenames = [
        ".env".to_owned(),
        format!(".env.{muddle_env}"),
        format!(".env.{package_name}"),
        format!(".env.{package_name}.{muddle_env}"),
    ];
    for filename in filenames {
        if let Ok(path) = dotenv::from_filename(filename) {
            log::info!(
                "Loaded environment variables from '{}'",
                path.to_string_lossy()
            );
        }
    }
}

#[macro_export]
macro_rules! try_parse_from_env {
    ($var_name:expr $(,)?) => {
        std::env::var($var_name)
            .ok()
            .map(|value| {
                log::info!(
                    "Reading {} from the environment variable: {}",
                    $var_name,
                    value
                );
                value
            })
            .or_else(|| {
                std::option_env!($var_name).map(str::to_owned).map(|value| {
                    log::info!(
                        "Reading {} from the compile-time environment variable: {}",
                        $var_name,
                        value
                    );
                    value
                })
            })
            .or_else(|| {
                log::warn!("Variable {} wasn't found", $var_name);
                None
            })
            .and_then(|value| {
                if value.is_empty() {
                    return None;
                }
                let parsed = value
                    .parse()
                    .ok()
                    .unwrap_or_else(|| panic!("Couldn't parse {} (value: {:?})", $var_name, value));
                Some(parsed)
            })
    };
}

#[macro_export]
macro_rules! var {
    ($var_name:expr $(,)?) => {
        std::env::var($var_name)
            .ok()
            .or_else(|| std::option_env!($var_name).map(str::to_owned))
            .and_then(|value| if value.is_empty() { None } else { Some(value) })
    };
}
