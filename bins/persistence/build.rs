use mr_build_dotenv::load_env;

fn main() {
    load_env();
    // Trigger recompilation when modifying migrations.
    println!("cargo:rerun-if-changed=migrations");
}
