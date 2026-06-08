use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use serde::Deserialize;
use tauri::State;

use yaminabe_launcher_shared::datatypes::JavaInstall;
use yaminabe_launcher_shared::error::Error;
use crate::{runtimes_dir, AppState};
use crate::http_utils::{download_resource, fetch_json};
// ── Local detection ───────────────────────────────────────────────────────────

fn get_java_version(jdk_dir: &Path) -> String {
    let release = jdk_dir.join("release");
    if let Ok(content) = std::fs::read_to_string(&release) {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("JAVA_VERSION=") {
                return rest.trim_matches('"').to_string();
            }
        }
    }
    String::from("unknown")
}

pub fn detect_java_installs() -> Vec<JavaInstall> {
    let mut installs: Vec<JavaInstall> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let vendor_dirs = [
        "Eclipse Adoptium", "Microsoft", "Eclipse Foundation",
        "Amazon Corretto", "Azul Systems", "BellSoft",
        "GraalVM", "Oracle", "Java", "OpenJDK",
    ];
    let program_files = [
        std::env::var("PROGRAMFILES").unwrap_or_else(|_| r"C:\Program Files".into()),
        std::env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into()),
    ];

    for root in &program_files {
        for vendor in &vendor_dirs {
            let vendor_path = Path::new(root).join(vendor);
            if let Ok(entries) = std::fs::read_dir(&vendor_path) {
                for entry in entries.flatten() {
                    let jdk = entry.path();
                    let key = jdk.to_string_lossy().to_string();
                    let exe = jdk.join("bin").join("javaw.exe");
                    if !exe.exists() { continue; }
                    if seen.contains(&key) { continue; }
                    seen.insert(key);
                    let version = get_java_version(&jdk);
                    installs.push(JavaInstall {
                        path: exe.to_string_lossy().into_owned(),
                        version,
                        vendor: (*vendor).to_string(),
                    });
                }
            }
        }
    }

    installs
}

#[tauri::command]
pub fn get_java_installs(state: State<'_, AppState>) -> Vec<JavaInstall> {
    state.java_installs.lock().unwrap().clone()
}

// ── Mojang JRE download ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct JavaRuntimeEntry {
    manifest: ManifestRef,
}

#[derive(Deserialize)]
struct ManifestRef {
    url: String,
}

#[derive(Deserialize)]
struct RuntimeManifest {
    files: HashMap<String, RuntimeFile>,
}

#[derive(Deserialize)]
struct RuntimeFile {
    #[serde(rename = "type")]
    file_type: String,
    #[serde(default)]
    downloads: Option<RuntimeFileDownloads>,
}

#[derive(Deserialize)]
struct RuntimeFileDownloads {
    raw: RuntimeDownload,
}

#[derive(Deserialize)]
struct RuntimeDownload {
    url: String,
    sha1: String,
}

/// Download a Mojang-distributed JRE for the given `component` (e.g. `"jre-legacy"`)
/// into `runtimes_dir/<component>/`. Returns the path to `javaw.exe`.
/// Skips download if `javaw.exe` already exists.
pub async fn download_java_runtime(
    component: &str,
    client: &reqwest::Client,
) -> Result<PathBuf, Error> {
    let runtime_dir = runtimes_dir().join(component);
    let javaw_path  = runtime_dir.join("bin").join("javaw.exe");

    let all = fetch_json(
        client,
        "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json",
    )
    .send::<HashMap<String, HashMap<String, Vec<JavaRuntimeEntry>>>>()
    .await?;

    let manifest_url = all
        .get("windows-x64")
        .and_then(|p| p.get(component))
        .and_then(|v| v.first())
        .map(|e| e.manifest.url.clone())
        .ok_or_else(|| Error::Invalid(format!("No Mojang JRE for component '{component}' on windows-x64")))?;

    let manifest = fetch_json(client, &manifest_url)
        .send::<RuntimeManifest>()
        .await?;

    std::fs::create_dir_all(&runtime_dir)?;

    for (rel_path, file) in &manifest.files {
        let dest = runtime_dir.join(rel_path);
        if file.file_type == "directory" {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if file.file_type == "file" && let Some(dl) = &file.downloads {
            download_resource(client, dl.raw.url.as_str(), &dl.raw.sha1, dest).await?;
        }
    }

    if !javaw_path.exists() {
        return Err(Error::NotExists(javaw_path.to_string_lossy().to_string()));
    }

    Ok(javaw_path)
}
