/// Build-time env keys forwarded from `.env` (or the shell) into `rustc`'s
/// environment so `option_env!` in the main crate picks them up.
const BUILD_ENV_KEYS: &[&str] = &["YAMINABE_AZURE_CLIENT_ID"];

fn main() {
    // Watch both common .env locations unconditionally so cargo re-runs
    // build.rs when a file is created (cargo treats missing paths as
    // "rerun when this appears"). Without this, a first build without
    // .env caches "no env var" and never re-checks.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    println!("cargo:rerun-if-changed={manifest_dir}/.env");
    println!("cargo:rerun-if-changed={manifest_dir}/../.env");

    dotenvy::dotenv().ok();

    for key in BUILD_ENV_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        if let Ok(value) = std::env::var(key) {
            println!("cargo:rustc-env={key}={value}");
        }
    }
    tauri_build::build()
}