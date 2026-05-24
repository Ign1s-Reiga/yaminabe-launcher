/// Build-time env keys forwarded from `.env` (or the shell) into `rustc`'s
/// environment so `option_env!` in the main crate picks them up.
const BUILD_ENV_KEYS: &[&str] = &["YAMINABE_AZURE_CLIENT_ID"];

fn main() {
    // dotenvy walks up from the package dir; a missing .env is fine — values
    // can also come from the shell env in CI or release builds.
    if let Ok(path) = dotenvy::dotenv() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for key in BUILD_ENV_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        if let Ok(value) = std::env::var(key) {
            println!("cargo:rustc-env={key}={value}");
        }
    }
    tauri_build::build()
}