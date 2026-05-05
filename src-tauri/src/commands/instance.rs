use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::info;
use tauri::State;

use yaminabe_launcher_shared::datatypes::{ModTool, InstanceMeta};
use crate::{emit_progress, AppState};
// ── Helpers ───────────────────────────────────────────────────────────────────


pub fn find_instance_dir(install_dir: &Path, id: &str) -> Option<PathBuf> {
    std::fs::read_dir(install_dir).ok()?.flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .find(|p| {
            std::fs::read_to_string(p.join("instance.json")).ok()
                .and_then(|s| serde_json::from_str::<InstanceMeta>(&s).ok())
                .map(|m| m.id == id)
                .unwrap_or(false)
        })
}


async fn ensure_vanilla(
    mc_version: &str,
    versions_dir: &Path,
    client: &reqwest::Client,
) -> Result<(), String> {
    let jar_path = versions_dir.join(mc_version).join(format!("{mc_version}.jar"));
    if jar_path.exists() {
        return Ok(());
    }

    let manifest: serde_json::Value = client
        .get("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")
        .send().await
        .map_err(|e| format!("Failed to fetch version manifest: {e}"))?
        .json().await
        .map_err(|e| format!("Failed to parse version manifest: {e}"))?;

    let version_url = manifest["versions"]
        .as_array().ok_or("Invalid manifest")?
        .iter()
        .find(|v| v["id"].as_str() == Some(mc_version))
        .ok_or_else(|| format!("Minecraft {mc_version} not found in manifest"))?
        ["url"].as_str().ok_or("Invalid version URL")?.to_string();

    let version_json: serde_json::Value = client
        .get(&version_url)
        .send().await
        .map_err(|e| format!("Failed to fetch version JSON: {e}"))?
        .json().await
        .map_err(|e| format!("Failed to parse version JSON: {e}"))?;

    let client_url = version_json["downloads"]["client"]["url"]
        .as_str().ok_or("Client URL missing from version JSON")?.to_string();

    let version_dir = versions_dir.join(mc_version);
    std::fs::create_dir_all(&version_dir)
        .map_err(|e| format!("Failed to create version dir: {e}"))?;

    let jar_bytes = client
        .get(&client_url)
        .send().await
        .map_err(|e| format!("Failed to download client jar: {e}"))?
        .bytes().await
        .map_err(|e| format!("Failed to read client jar: {e}"))?;

    std::fs::write(&jar_path, &jar_bytes)
        .map_err(|e| format!("Failed to write client jar: {e}"))?;
    std::fs::write(
        version_dir.join(format!("{mc_version}.json")),
        serde_json::to_string_pretty(&version_json).unwrap(),
    ).map_err(|e| format!("Failed to write version JSON: {e}"))?;

    info!("Downloaded vanilla Minecraft {mc_version}");
    Ok(())
}

async fn ensure_fabric(
    mc_version: &str,
    loader_version_hint: Option<&str>,
    versions_dir: &Path,
    client: &reqwest::Client,
) -> Result<(), String> {
    let loader_version = if let Some(hint) = loader_version_hint {
        hint.strip_prefix("fabric-").unwrap_or(hint).to_string()
    } else {
        let loaders: serde_json::Value = client
            .get(format!("https://meta.fabricmc.net/v2/versions/loader/{mc_version}"))
            .send().await
            .map_err(|e| format!("Failed to fetch Fabric loader list: {e}"))?
            .json().await
            .map_err(|e| format!("Failed to parse Fabric loader list: {e}"))?;

        loaders.as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|v| v["loader"]["stable"].as_bool() == Some(true))
                    .or_else(|| arr.first())
            })
            .and_then(|v| v["loader"]["version"].as_str())
            .ok_or_else(|| format!("No Fabric loader found for MC {mc_version}"))?
            .to_string()
    };

    let version_id = format!("fabric-loader-{loader_version}-{mc_version}");
    let version_dir = versions_dir.join(&version_id);
    if version_dir.join(format!("{version_id}.jar")).exists() {
        info!("Fabric {loader_version} already installed, skipping");
        return Ok(());
    }
    std::fs::create_dir_all(&version_dir)
        .map_err(|e| format!("Failed to create Fabric version dir: {e}"))?;

    let profile: serde_json::Value = client
        .get(format!(
            "https://meta.fabricmc.net/v2/versions/loader/{mc_version}/{loader_version}/profile/json"
        ))
        .send().await
        .map_err(|e| format!("Failed to fetch Fabric profile: {e}"))?
        .json().await
        .map_err(|e| format!("Failed to parse Fabric profile: {e}"))?;

    std::fs::write(
        version_dir.join(format!("{version_id}.json")),
        serde_json::to_string_pretty(&profile).unwrap(),
    ).map_err(|e| format!("Failed to write Fabric profile JSON: {e}"))?;

    let jar_bytes = client
        .get(format!(
            "https://maven.fabricmc.net/net/fabricmc/fabric-loader/{loader_version}/fabric-loader-{loader_version}.jar"
        ))
        .send().await
        .map_err(|e| format!("Failed to download Fabric loader jar: {e}"))?
        .bytes().await
        .map_err(|e| format!("Failed to read Fabric loader jar: {e}"))?;

    std::fs::write(version_dir.join(format!("{version_id}.jar")), &jar_bytes)
        .map_err(|e| format!("Failed to write Fabric loader jar: {e}"))?;

    info!("Installed Fabric {loader_version} for MC {mc_version}");
    Ok(())
}

async fn ensure_quilt(
    mc_version: &str,
    loader_version_hint: Option<&str>,
    versions_dir: &Path,
    client: &reqwest::Client,
) -> Result<(), String> {
    let loader_version = if let Some(hint) = loader_version_hint {
        hint.strip_prefix("quilt-").unwrap_or(hint).to_string()
    } else {
        let loaders: serde_json::Value = client
            .get(format!("https://meta.quiltmc.org/v3/versions/loader/{mc_version}"))
            .send().await
            .map_err(|e| format!("Failed to fetch Quilt loader list: {e}"))?
            .json().await
            .map_err(|e| format!("Failed to parse Quilt loader list: {e}"))?;

        loaders.as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v["loader"]["version"].as_str())
            .ok_or_else(|| format!("No Quilt loader found for MC {mc_version}"))?
            .to_string()
    };

    let version_id = format!("quilt-loader-{loader_version}-{mc_version}");
    let version_dir = versions_dir.join(&version_id);
    if version_dir.join(format!("{version_id}.jar")).exists() {
        info!("Quilt {loader_version} already installed, skipping");
        return Ok(());
    }
    std::fs::create_dir_all(&version_dir)
        .map_err(|e| format!("Failed to create Quilt version dir: {e}"))?;

    let profile: serde_json::Value = client
        .get(format!(
            "https://meta.quiltmc.org/v3/versions/loader/{mc_version}/{loader_version}/profile/json"
        ))
        .send().await
        .map_err(|e| format!("Failed to fetch Quilt profile: {e}"))?
        .json().await
        .map_err(|e| format!("Failed to parse Quilt profile: {e}"))?;

    std::fs::write(
        version_dir.join(format!("{version_id}.json")),
        serde_json::to_string_pretty(&profile).unwrap(),
    ).map_err(|e| format!("Failed to write Quilt profile JSON: {e}"))?;

    let jar_bytes = client
        .get(format!(
            "https://maven.quiltmc.org/repository/release/org/quiltmc/quilt-loader/{loader_version}/quilt-loader-{loader_version}.jar"
        ))
        .send().await
        .map_err(|e| format!("Failed to download Quilt loader jar: {e}"))?
        .bytes().await
        .map_err(|e| format!("Failed to read Quilt loader jar: {e}"))?;

    std::fs::write(version_dir.join(format!("{version_id}.jar")), &jar_bytes)
        .map_err(|e| format!("Failed to write Quilt loader jar: {e}"))?;

    info!("Installed Quilt {loader_version} for MC {mc_version}");
    Ok(())
}

async fn ensure_forge(
    mc_version: &str,
    forge_hint: Option<&str>,
    versions_dir: &Path,
    java: &str,
    client: &reqwest::Client,
) -> Result<(), String> {
    let forge_build = if let Some(hint) = forge_hint {
        hint.strip_prefix("forge-").unwrap_or(hint).to_string()
    } else {
        let promos: serde_json::Value = client
            .get("https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")
            .send().await
            .map_err(|e| format!("Failed to fetch Forge promotions: {e}"))?
            .json().await
            .map_err(|e| format!("Failed to parse Forge promotions: {e}"))?;

        let map = promos["promos"].as_object().ok_or("Invalid Forge promotions format")?;
        map
            .get(&format!("{mc_version}-recommended"))
            .or_else(|| map.get(&format!("{mc_version}-latest")))
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("No Forge build found for MC {mc_version}"))?
            .to_string()
    };

    let version_id = format!("{mc_version}-forge-{forge_build}");
    if versions_dir.join(&version_id).exists() {
        info!("Forge {forge_build} already installed, skipping");
        return Ok(());
    }

    let installer_url = format!(
        "https://maven.minecraftforge.net/net/minecraftforge/forge/{mc_version}-{forge_build}/forge-{mc_version}-{forge_build}-installer.jar"
    );
    let jar_bytes = client
        .get(&installer_url)
        .send().await
        .map_err(|e| format!("Failed to download Forge installer: {e}"))?
        .bytes().await
        .map_err(|e| format!("Failed to read Forge installer: {e}"))?;

    let mc_dir = versions_dir.parent().ok_or("Could not determine Minecraft directory")?;
    let installer_path = mc_dir.join(format!("forge-{mc_version}-{forge_build}-installer.jar"));
    std::fs::write(&installer_path, &jar_bytes)
        .map_err(|e| format!("Failed to write Forge installer: {e}"))?;

    let status = tokio::process::Command::new(java)
        .args([
            "-jar",
            installer_path.to_str().unwrap_or_default(),
            "--installClient",
            mc_dir.to_str().unwrap_or_default(),
        ])
        .status().await
        .map_err(|e| format!("Failed to run Forge installer: {e}"))?;

    let _ = std::fs::remove_file(&installer_path);
    if !status.success() {
        return Err(format!("Forge installer exited with {status}"));
    }

    info!("Installed Forge {forge_build} for MC {mc_version}");
    Ok(())
}

async fn ensure_neoforge(
    mc_version: &str,
    nf_hint: Option<&str>,
    versions_dir: &Path,
    java: &str,
    client: &reqwest::Client,
) -> Result<(), String> {
    let nf_version = if let Some(hint) = nf_hint {
        hint.strip_prefix("neoforge-").unwrap_or(hint).to_string()
    } else {
        // NeoForge version prefix: "1.21.4" → "21.4"
        let nf_prefix = mc_version.strip_prefix("1.").unwrap_or(mc_version);

        let xml = client
            .get("https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml")
            .send().await
            .map_err(|e| format!("Failed to fetch NeoForge metadata: {e}"))?
            .text().await
            .map_err(|e| format!("Failed to read NeoForge metadata: {e}"))?;

        xml
            .split("<version>").skip(1)
            .filter_map(|chunk| chunk.split("</version>").next())
            .filter(|v| v.starts_with(&format!("{nf_prefix}.")))
            .last()
            .ok_or_else(|| format!("No NeoForge version found for MC {mc_version}"))?
            .to_string()
    };

    let version_id = format!("neoforge-{nf_version}");
    if versions_dir.join(&version_id).exists() {
        info!("NeoForge {nf_version} already installed, skipping");
        return Ok(());
    }

    let installer_url = format!(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/{nf_version}/neoforge-{nf_version}-installer.jar"
    );
    let jar_bytes = client
        .get(&installer_url)
        .send().await
        .map_err(|e| format!("Failed to download NeoForge installer: {e}"))?
        .bytes().await
        .map_err(|e| format!("Failed to read NeoForge installer: {e}"))?;

    let mc_dir = versions_dir.parent().ok_or("Could not determine Minecraft directory")?;
    let installer_path = mc_dir.join(format!("neoforge-{nf_version}-installer.jar"));
    std::fs::write(&installer_path, &jar_bytes)
        .map_err(|e| format!("Failed to write NeoForge installer: {e}"))?;

    let status = tokio::process::Command::new(java)
        .args([
            "-jar",
            installer_path.to_str().unwrap_or_default(),
            "--install-client",
            mc_dir.to_str().unwrap_or_default(),
        ])
        .status().await
        .map_err(|e| format!("Failed to run NeoForge installer: {e}"))?;

    let _ = std::fs::remove_file(&installer_path);
    if !status.success() {
        return Err(format!("NeoForge installer exited with {status}"));
    }

    info!("Installed NeoForge {nf_version} for MC {mc_version}");
    Ok(())
}

// ── Mod downloads ─────────────────────────────────────────────────────────────

async fn download_mods_modrinth(
    version_ids: &[String],
    instance_location: &str,
    client: &reqwest::Client,
) -> Result<(), String> {
    if version_ids.is_empty() {
        return Ok(());
    }

    let mods_dir = PathBuf::from(instance_location).join("mods");
    std::fs::create_dir_all(&mods_dir)
        .map_err(|e| format!("Failed to create mods directory: {e}"))?;

    let ids_json = serde_json::to_string(version_ids).unwrap();
    let versions: serde_json::Value = client
        .get("https://api.modrinth.com/v2/versions")
        .query(&[("ids", ids_json.as_str())])
        .send().await
        .map_err(|e| format!("Failed to fetch Modrinth versions: {e}"))?
        .json().await
        .map_err(|e| format!("Failed to parse Modrinth versions: {e}"))?;

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<(), String>>> = Vec::new();

    for version in versions.as_array().unwrap_or(&vec![]) {
        let files = version["files"].as_array();
        let file = files.and_then(|f| {
            f.iter().find(|e| e["primary"].as_bool() == Some(true)).or_else(|| f.first())
        });
        let Some(file) = file else { continue };

        let url      = file["url"].as_str().unwrap_or_default().to_string();
        let filename = file["filename"].as_str().unwrap_or_default().to_string();
        if url.is_empty() || filename.is_empty() { continue }

        let client   = client.clone();
        let mods_dir = mods_dir.clone();
        let sem      = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await
                .map_err(|e| format!("Semaphore error: {e}"))?;
            let resp = client.get(&url).send().await
                .map_err(|e| format!("Download failed for {filename}: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Download of {filename} returned {}", resp.status()));
            }
            let bytes = resp.bytes().await
                .map_err(|e| format!("Failed to read {filename}: {e}"))?;
            std::fs::write(mods_dir.join(&filename), &bytes)
                .map_err(|e| format!("Failed to write {filename}: {e}"))?;
            info!("Downloaded {filename}");
            Ok(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| format!("Download task failed: {e}"))??;
    }

    Ok(())
}

#[tauri::command]
pub async fn download_mods(
    file_ids: Vec<String>,
    instance_location: String,
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    match source.as_deref().unwrap_or("modrinth") {
        "curseforge" => {
            let ids: Vec<u32> = file_ids.iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();
            crate::commands::curseforge::download_mods_core(
                ids, &instance_location, &api_key, &state.http_client,
            ).await
        }
        _ => download_mods_modrinth(&file_ids, &instance_location, &state.http_client).await,
    }
}

// ── Command ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_instance(
    app_handle: tauri::AppHandle,
    instance_name: String,
    instance_location: String,
    mc_version: String,
    mod_tool: ModTool,
    mod_tool_version: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string();

    macro_rules! step {
        ($s:expr) => { crate::emit_progress(&app_handle, &id, &instance_name, $s, false, None); };
    }
    macro_rules! fail {
        ($e:expr) => {{
            crate::emit_progress(&app_handle, &id, &instance_name, "Failed", false, Some($e.clone()));
            return Err($e);
        }};
    }

    step!("Preparing directories");
    let instance_path = PathBuf::from(&instance_location).join(instance_name.to_lowercase());
    if instance_path.exists() {
        let e = format!("Folder '{}' already exists at this location", instance_name.to_lowercase());
        fail!(e);
    }
    if let Err(e) = std::fs::create_dir_all(&instance_path)
        .map_err(|e| format!("Failed to create instance directory: {e}"))
    { fail!(e); }

    let versions_dir: &Path = &state.versions_dir;
    let libraries_dir: &Path = &state.libraries_dir;
    for dir in [versions_dir, libraries_dir] {
        if let Err(e) = std::fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create directory: {e}"))
        { fail!(e); }
    }

    step!(&format!("Downloading Minecraft {mc_version}"));
    if let Err(e) = ensure_vanilla(&mc_version, &versions_dir, &state.http_client).await { fail!(e); }

    // Read javaVersion.component from the downloaded version JSON
    let java_component = {
        #[derive(serde::Deserialize, Default)]
        struct JavaVersion { component: String }
        #[derive(serde::Deserialize, Default)]
        struct VersionJson {
            #[serde(rename = "javaVersion", default)]
            java_version: JavaVersion,
        }
        let json_path = versions_dir.join(&mc_version).join(format!("{mc_version}.json"));
        std::fs::read_to_string(&json_path).ok()
            .and_then(|s| serde_json::from_str::<VersionJson>(&s).ok())
            .map(|v| v.java_version.component)
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "java-runtime-epsilon".to_string())
    };

    step!(&format!("Downloading Java ({java_component})"));
    let runtimes_dir: &Path = &state.runtimes_dir;
    let java = match crate::commands::java::download_java_runtime(&java_component, runtimes_dir, &state.http_client).await {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(e) => {
            // Non-fatal: fall back to system javaw
            log::warn!("JRE download failed ({e}), falling back to system javaw");
            "javaw".to_string()
        }
    };

    let hint = mod_tool_version.as_deref();
    match &mod_tool {
        ModTool::Fabric => {
            step!("Installing Fabric");
            if let Err(e) = ensure_fabric(&mc_version, hint, &versions_dir, &state.http_client).await { fail!(e); }
        }
        ModTool::Quilt => {
            step!("Installing Quilt");
            if let Err(e) = ensure_quilt(&mc_version, hint, &versions_dir, &state.http_client).await { fail!(e); }
        }
        ModTool::Forge => {
            step!("Installing Forge");
            if let Err(e) = ensure_forge(&mc_version, hint, &versions_dir, &java, &state.http_client).await { fail!(e); }
        }
        ModTool::NeoForge => {
            step!("Installing NeoForge");
            if let Err(e) = ensure_neoforge(&mc_version, hint, &versions_dir, &java, &state.http_client).await { fail!(e); }
        }
        ModTool::Vanilla => {}
    }

    step!("Finalizing");
    let meta = InstanceMeta {
        id: id.clone(),
        name: instance_name.clone(),
        mc_version,
        mod_tool: mod_tool.to_string(),
        mod_tool_version,
        category: String::new(),
        ram_mb: 4096,
        jvm_args: String::new(),
        jre_path: java,
        description: String::new(),
        window_width: 0,
        window_height: 0,
    };
    if let Err(e) = std::fs::write(
        instance_path.join("instance.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    ).map_err(|e| format!("Failed to write instance.json: {e}")) { fail!(e); }

    emit_progress(&app_handle, &meta.id, &instance_name, "Done", true, None);
    info!("Created '{}' (MC {}, {}) → {}", instance_name, meta.mc_version, meta.mod_tool, instance_path.display());
    Ok(())
}

#[tauri::command]
pub async fn get_instances(state: State<'_, AppState>) -> Result<Vec<crate::InstanceMeta>, String> {
    let location = state.settings.lock().unwrap().instance_install_dir.clone();
    if location.is_empty() {
        return Ok(vec![]);
    }
    let root = PathBuf::from(&location);
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut instances = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let json_path = path.join("instance.json");
        if !json_path.exists() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&json_path) else { continue };
        let Ok(meta) = serde_json::from_str::<InstanceMeta>(&content) else { continue };
        instances.push(meta);
    }
    instances.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(instances)
}

#[tauri::command]
pub fn save_instance_settings(
    id: String,
    ram_mb: u32,
    jvm_args: String,
    jre_path: String,
    description: String,
    window_width: u32,
    window_height: u32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let install_dir = state.settings.lock().unwrap().instance_install_dir.clone();
    let dir = find_instance_dir(Path::new(&install_dir), &id)
        .ok_or_else(|| format!("Instance '{id}' not found"))?;
    let json_path = dir.join("instance.json");
    let content = std::fs::read_to_string(&json_path)
        .map_err(|e| format!("Read error: {e}"))?;
    let mut meta: InstanceMeta = serde_json::from_str(&content)
        .map_err(|e| format!("Parse error: {e}"))?;

    meta.ram_mb = ram_mb;
    meta.jvm_args = jvm_args;
    meta.jre_path = jre_path;
    meta.description = description;
    meta.window_width = window_width;
    meta.window_height = window_height;

    std::fs::write(&json_path, serde_json::to_string_pretty(&meta).unwrap())
        .map_err(|e| format!("Write error: {e}"))?;
    Ok(())
}
