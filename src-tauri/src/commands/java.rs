use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use serde::Deserialize;
use tauri::State;

use yaminabe_launcher_shared::datatypes::JavaInstall;
use crate::{runtimes_dir, AppState};

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
}

/// Download a Mojang-distributed JRE for the given `component` (e.g. `"jre-legacy"`)
/// into `runtimes_dir/<component>/`. Returns the path to `javaw.exe`.
/// Skips download if `javaw.exe` already exists.
pub async fn download_java_runtime(
    component: &str,
    client: &reqwest::Client,
) -> Result<PathBuf, String> {
    let runtime_dir = runtimes_dir().join(component);
    let javaw_path  = runtime_dir.join("bin").join("javaw.exe");
    if javaw_path.exists() {
        return Ok(javaw_path);
    }

    let all: HashMap<String, HashMap<String, Vec<JavaRuntimeEntry>>> = client
        .get("https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json")
        .send().await
        .map_err(|e| format!("Failed to fetch Mojang JRE list: {e}"))?
        .json().await
        .map_err(|e| format!("Failed to parse Mojang JRE list: {e}"))?;

    let manifest_url = all
        .get("windows-x64")
        .and_then(|p| p.get(component))
        .and_then(|v| v.first())
        .map(|e| e.manifest.url.clone())
        .ok_or_else(|| format!("No Mojang JRE for component '{component}' on windows-x64"))?;

    let manifest: RuntimeManifest = client
        .get(&manifest_url)
        .send().await
        .map_err(|e| format!("Failed to fetch JRE manifest: {e}"))?
        .json().await
        .map_err(|e| format!("Failed to parse JRE manifest: {e}"))?;

    std::fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("Failed to create runtime dir: {e}"))?;

    for (rel_path, file) in &manifest.files {
        let dest = runtime_dir.join(rel_path);
        if file.file_type == "directory" {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("Failed to create dir '{rel_path}': {e}"))?;
            continue;
        }
        if file.file_type != "file" { continue; }
        let Some(dl) = &file.downloads else { continue };
        if dest.exists() { continue; }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent for '{rel_path}': {e}"))?;
        }
        let bytes = client.get(&dl.raw.url).send().await
            .map_err(|e| format!("Failed to download '{rel_path}': {e}"))?
            .bytes().await
            .map_err(|e| format!("Failed to read '{rel_path}': {e}"))?;
        std::fs::write(&dest, &bytes)
            .map_err(|e| format!("Failed to write '{rel_path}': {e}"))?;
    }

    if !javaw_path.exists() {
        return Err(format!("javaw.exe not found after downloading runtime '{component}'"));
    }

    Ok(javaw_path)
}
