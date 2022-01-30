pub fn load_env() {
    println!("cargo:rerun-if-env-changed=MUDDLE_ENV");
    // Sync this with the dotenv module of `utils_lib`.
    let muddle_env = std::env::var("MUDDLE_ENV").unwrap_or_else(|_| {
        match std::env::var("PROFILE").as_deref() {
            Ok("release") => "production",
            Ok("debug") => "development",
            _ => "development",
        }
        .to_owned()
    });
    let package_name = std::env::var("CARGO_PKG_NAME").unwrap();
    let filenames = [
        ".env".to_owned(),
        format!(".env.{muddle_env}"),
        format!(".env.{package_name}"),
        format!(".env.{package_name}.{muddle_env}"),
    ];
    for filename in filenames {
        if let Ok((path, vars)) = dotenv::from_filename_iter(filename) {
            println!(
                "Loaded environment variables from '{}'",
                path.to_string_lossy()
            );
            for entry in vars {
                let (key, value) = entry.expect("Failed to parse a dotenv file entry");
                println!("cargo:rustc-env={}={}", key, value);
            }
            println!("cargo:rerun-if-changed={}", path.to_string_lossy());
        }
    }
}
