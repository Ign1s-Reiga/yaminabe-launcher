use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use log::info;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use yaminabe_launcher_shared::datatypes::InstanceMeta;
use crate::AppState;
use crate::commands::instance::find_instance_dir;
// ── IPC event ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub instance_id: String,
    pub line: String,
    pub done: bool,
    pub error: Option<String>,
}

// ── Version JSON types ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct VersionManifest {
    #[serde(rename = "mainClass", default)]
    main_class: String,
    #[serde(default)]
    arguments: Option<Arguments>,
    #[serde(rename = "assetIndex")]
    asset_index: Option<AssetIndex>,
    #[serde(default)]
    libraries: Vec<Library>,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
    #[serde(rename = "inheritsFrom")]
    inherits_from: Option<String>,
}

#[derive(Deserialize)]
struct Arguments {
    #[serde(rename = "default-user-jvm", default)]
    default_user_jvm: Vec<DefaultJvmItem>,
    #[serde(default)]
    game: Vec<ArgumentItem>,
    #[serde(default)]
    jvm: Vec<ArgumentItem>,
}

#[derive(Deserialize)]
struct AssetIndex {
    id: String,
}

#[derive(Deserialize)]
struct Library {
    name: String,
    #[serde(default)]
    downloads: Option<LibraryDownloads>,
    #[serde(default)]
    rules: Vec<LibRule>,
    #[serde(default)]
    natives: HashMap<String, String>,
    url: Option<String>,
}

#[derive(Deserialize)]
struct LibraryDownloads {
    artifact: Option<LibraryArtifact>,
    classifiers: Option<HashMap<String, LibraryArtifact>>,
}

#[derive(Deserialize)]
struct LibraryArtifact {
    path: Option<String>,
    url: String,
}

#[derive(Deserialize)]
struct LibRule {
    action: String,
    #[serde(default)]
    os: LibRuleOs,
}

#[derive(Deserialize, Default)]
struct LibRuleOs {
    name: Option<String>,
}

// ── Argument item types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(untagged)]
enum ArgValue {
    One(String),
    Many(Vec<String>),
}


#[derive(Deserialize)]
struct VersionRange {
    min: Option<String>,
    max: Option<String>,
}

#[derive(Deserialize, Default)]
struct ArgRuleOs {
    name: Option<String>,
    arch: Option<String>,
    #[serde(rename = "versionRange")]
    version_range: Option<VersionRange>,
}

#[derive(Deserialize, Default)]
struct ArgFeatures {
    #[serde(default)]
    has_custom_resolution: bool,
    #[serde(default)]
    is_demo_user: bool,
    #[serde(default)]
    has_quick_plays_support: bool,
    #[serde(default)]
    is_quick_play_singleplayer: bool,
    #[serde(default)]
    is_quick_play_multiplayer: bool,
    #[serde(default)]
    is_quick_play_realms: bool,
}

#[derive(Deserialize)]
struct ArgRule {
    action: String,
    #[serde(default)]
    os: ArgRuleOs,
    #[serde(default)]
    features: Option<ArgFeatures>,
}

#[derive(Deserialize)]
struct DefaultJvmItem {
    #[serde(default)]
    rules: Vec<ArgRule>,
    value: ArgValue,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ArgumentItem {
    Plain(String),
    Conditional {
        #[serde(default)]
        rules: Vec<ArgRule>,
        value: ArgValue,
    },
}

// ── Helper functions ──────────────────────────────────────────────────────────

fn find_version_id(versions_dir: &Path, mc_version: &str, mod_tool: &str) -> String {
    if mod_tool == "Vanilla" {
        return mc_version.to_string();
    }
    let prefix = match mod_tool {
        "Fabric" => "fabric-loader-".to_string(),
        "Quilt" => "quilt-loader-".to_string(),
        "Forge" => format!("{mc_version}-forge-"),
        "NeoForge" => "neoforge-".to_string(),
        _  => return mc_version.to_string(),
    };
    let Ok(entries) = std::fs::read_dir(versions_dir) else {
        return mc_version.to_string();
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) {
            let loader_needs_mc = matches!(mod_tool, "Fabric" | "Quilt");
            if !loader_needs_mc || name.ends_with(mc_version) {
                return name;
            }
        }
    }
    mc_version.to_string()
}

fn load_manifest(versions_dir: &Path, version_id: &str) -> Result<VersionManifest, String> {
    let path = versions_dir.join(version_id).join(format!("{version_id}.json"));
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read {path:?}: {e}"))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("Cannot parse {path:?}: {e}"))
}

/// Load and merge a version manifest, resolving `inheritsFrom` one level deep.
/// Downloads the JSON (+ client) from Mojang if not found on disk.
/// Child fields take priority; libraries are concatenated (child first).
async fn merged_manifest(
    versions_dir: &Path,
    version_id: &str,
    client: &reqwest::Client,
) -> Result<VersionManifest, String> {
    let _ = ensure_version_json(versions_dir, version_id, client).await;
    let mut manifest = load_manifest(versions_dir, version_id)?;
    let parent_id = manifest.inherits_from.take();
    if let Some(parent_id) = parent_id {
        let _ = ensure_version_json(versions_dir, &parent_id, client).await;
        let parent = load_manifest(versions_dir, &parent_id)?;
        let child_names: HashSet<String> = manifest.libraries.iter().map(|l| l.name.clone()).collect();
        for lib in parent.libraries {
            if !child_names.contains(&lib.name) {
                manifest.libraries.push(lib);
            }
        }
        if manifest.arguments.is_none()          { manifest.arguments          = parent.arguments;          }
        if manifest.asset_index.is_none()         { manifest.asset_index         = parent.asset_index;         }
        if manifest.main_class.is_empty()         { manifest.main_class          = parent.main_class;          }
        if manifest.minecraft_arguments.is_none() { manifest.minecraft_arguments = parent.minecraft_arguments; }
    }
    Ok(manifest)
}

fn maven_to_path(name: &str) -> Option<String> {
    let mut parts = name.splitn(4, ':');
    let group    = parts.next()?.replace('.', "/");
    let artifact = parts.next()?;
    let version  = parts.next()?;
    let suffix   = match parts.next() {
        Some(cls) => format!("{artifact}-{version}-{cls}.jar"),
        None      => format!("{artifact}-{version}.jar"),
    };
    Some(format!("{group}/{artifact}/{version}/{suffix}"))
}

fn os_allowed(rules: &[LibRule]) -> bool {
    if rules.is_empty() { return true; }
    let mut result = false;
    for rule in rules {
        let os_ok = rule.os.name.as_deref().map_or(true, |n| n == "windows");
        if os_ok { result = rule.action == "allow"; }
    }
    result
}

async fn ensure_library(
    path_rel: &str,
    url: &str,
    libraries_dir: &Path,
    client: &reqwest::Client,
) -> Result<(), String> {
    let dest = libraries_dir.join(path_rel);
    if dest.exists() { return Ok(()); }
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let bytes = client.get(url).send().await
        .map_err(|e| format!("GET {url}: {e}"))?
        .bytes().await
        .map_err(|e| format!("read body {url}: {e}"))?;
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())
}

async fn download_libraries(
    libraries: &[Library],
    libraries_dir: &Path,
    client: &reqwest::Client,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for lib in libraries {
        if !os_allowed(&lib.rules) { continue; }
        let ok = if let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            if let Some(path) = &artifact.path {
                ensure_library(path, &artifact.url, libraries_dir, client).await
            } else { Ok(()) }
        } else if let Some(rel) = maven_to_path(&lib.name) {
            let base = lib.url.as_deref().unwrap_or("https://libraries.minecraft.net/");
            let url  = format!("{}/{}", base.trim_end_matches('/'), rel);
            ensure_library(&rel, &url, libraries_dir, client).await
        } else { Ok(()) };
        if let Err(e) = ok { warnings.push(e); }
    }
    warnings
}

fn extract_natives(
    libraries: &[Library],
    libraries_dir: &Path,
    natives_dir: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(natives_dir).map_err(|e| e.to_string())?;
    for lib in libraries {
        if !os_allowed(&lib.rules) { continue; }
        let key = lib.natives.get("windows").map(|s| s.replace("${arch}", "64"));
        let Some(key) = key else { continue; };
        let Some(classifiers) = lib.downloads.as_ref().and_then(|d| d.classifiers.as_ref()) else { continue; };
        let Some(artifact) = classifiers.get(&key) else { continue; };
        let Some(path) = &artifact.path else { continue; };
        let jar_path = libraries_dir.join(path);
        if !jar_path.exists() { continue; }
        let Ok(file)    = std::fs::File::open(&jar_path) else { continue; };
        let Ok(mut zip) = zip::ZipArchive::new(file) else { continue; };
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else { continue; };
            let name = entry.name().to_string();
            if !name.ends_with(".dll") && !name.ends_with(".so") && !name.ends_with(".dylib") { continue; }
            let dest = natives_dir.join(&name);
            if dest.exists() { continue; }
            let Ok(mut f) = std::fs::File::create(&dest) else { continue; };
            let _ = std::io::copy(&mut entry, &mut f);
        }
    }
    Ok(())
}

fn build_classpath(
    libraries: &[Library],
    libraries_dir: &Path,
    versions_dir: &Path,
    version_id: &str,
) -> String {
    let mut paths: Vec<String> = Vec::new();
    for lib in libraries {
        if !os_allowed(&lib.rules) { continue; }
        // Skip native-only entries (no artifact but has natives classifiers)
        let has_artifact = lib.downloads.as_ref().map_or(true, |d| d.artifact.is_some());
        if !has_artifact && !lib.natives.is_empty() { continue; }
        let p = if let Some(path) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()).and_then(|a| a.path.as_deref()) {
            Some(libraries_dir.join(path))
        } else if let Some(rel) = maven_to_path(&lib.name) {
            Some(libraries_dir.join(rel))
        } else { None };
        if let Some(p) = p {
            if p.exists() { paths.push(p.to_string_lossy().into_owned()); }
        }
    }
    let jar = versions_dir.join(version_id).join(format!("{version_id}.jar"));
    if jar.exists() { paths.push(jar.to_string_lossy().into_owned()); }
    paths.join(";")
}

struct LaunchVars<'a> {
    natives_directory: &'a str,
    classpath:         &'a str,
    launcher_name:     &'a str,
    launcher_version:  &'a str,
    auth_player_name:  &'a str,
    version_name:      &'a str,
    game_directory:    &'a str,
    assets_root:       &'a str,
    assets_index_name: &'a str,
    auth_uuid:         &'a str,
    auth_access_token: &'a str,
    user_type:         &'a str,
    version_type:      &'a str,
    clientid:          &'a str,
    auth_xuid:         &'a str,
    resolution_width:  &'a str,
    resolution_height: &'a str,
}

fn substitute_vars(s: &str, v: &LaunchVars) -> String {
    s.replace("${natives_directory}", v.natives_directory)
     .replace("${classpath}",         v.classpath)
     .replace("${launcher_name}",     v.launcher_name)
     .replace("${launcher_version}",  v.launcher_version)
     .replace("${auth_player_name}",  v.auth_player_name)
     .replace("${version_name}",      v.version_name)
     .replace("${game_directory}",    v.game_directory)
     .replace("${assets_root}",       v.assets_root)
     .replace("${assets_index_name}", v.assets_index_name)
     .replace("${auth_uuid}",         v.auth_uuid)
     .replace("${auth_access_token}", v.auth_access_token)
     .replace("${user_type}",         v.user_type)
     .replace("${version_type}",      v.version_type)
     .replace("${clientid}",          v.clientid)
     .replace("${auth_xuid}",         v.auth_xuid)
     .replace("${resolution_width}",  v.resolution_width)
     .replace("${resolution_height}", v.resolution_height)
}

fn eval_rules(rules: &[ArgRule]) -> bool {
    if rules.is_empty() { return true; }
    let mut result = false;
    for rule in rules {
        let os_ok = if let Some(vr) = &rule.os.version_range {
            vr.min.is_some() && vr.max.is_none()
        } else {
            rule.os.name.as_deref().map_or(true, |n| n == "windows")
        };
        let arch_ok = rule.os.arch.as_deref().map_or(true, |a| a != "x86");
        if os_ok && arch_ok { result = rule.action == "allow"; }
    }
    result
}

fn collect_default_jvm(items: &[DefaultJvmItem]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if item.rules.iter().any(|r| r.features.is_some()) { continue; }
        if !eval_rules(&item.rules) { continue; }
        match &item.value {
            ArgValue::One(s)  => out.push(s.clone()),
            ArgValue::Many(v) => out.extend(v.iter().cloned()),
        }
    }
    out
}

fn process_args(items: &[ArgumentItem], vars: &LaunchVars) -> Vec<String> {
    let has_resolution = !vars.resolution_width.is_empty() && !vars.resolution_height.is_empty();
    let mut out = Vec::new();
    for item in items {
        match item {
            ArgumentItem::Plain(s) => out.push(substitute_vars(s, vars)),
            ArgumentItem::Conditional { rules, value } => {
                let feature_applies = rules.iter().any(|r| {
                    r.features.as_ref().map_or(false, |f| f.has_custom_resolution)
                });
                if feature_applies {
                    if has_resolution {
                        match value {
                            ArgValue::One(s)  => out.push(substitute_vars(s, vars)),
                            ArgValue::Many(v) => out.extend(v.iter().map(|s| substitute_vars(s, vars))),
                        }
                    }
                    continue;
                }
                if rules.iter().any(|r| r.features.is_some()) { continue; }
                if !eval_rules(rules) { continue; }
                match value {
                    ArgValue::One(s)  => out.push(substitute_vars(s, vars)),
                    ArgValue::Many(v) => out.extend(v.iter().map(|s| substitute_vars(s, vars))),
                }
            }
        }
    }
    out
}

async fn ensure_version_json(
    versions_dir: &Path,
    version_id: &str,
    client: &reqwest::Client,
) -> Result<(), String> {
    let path = versions_dir.join(version_id).join(format!("{version_id}.json"));
    if path.exists() { return Ok(()); }

    let manifest: serde_json::Value = client
        .get("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")
        .send().await.map_err(|e| format!("Failed to fetch manifest: {e}"))?
        .json().await.map_err(|e| format!("Failed to parse manifest: {e}"))?;

    let version_url = manifest["versions"]
        .as_array().ok_or("Invalid manifest format")?
        .iter()
        .find(|v| v["id"].as_str() == Some(version_id))
        .ok_or_else(|| format!("{version_id} not found in Mojang manifest"))?
        ["url"].as_str().ok_or("Invalid version URL")?.to_string();

    let version_json: serde_json::Value = client
        .get(&version_url)
        .send().await.map_err(|e| format!("Failed to fetch version JSON: {e}"))?
        .json().await.map_err(|e| format!("Failed to parse version JSON: {e}"))?;

    let version_dir = versions_dir.join(version_id);
    std::fs::create_dir_all(&version_dir)
        .map_err(|e| format!("Failed to create version dir: {e}"))?;
    std::fs::write(&path, serde_json::to_string_pretty(&version_json).unwrap())
        .map_err(|e| format!("Failed to write version JSON: {e}"))?;

    if let Some(client_url) = version_json["downloads"]["client"]["url"].as_str() {
        let jar_path = version_dir.join(format!("{version_id}.jar"));
        if !jar_path.exists() {
            let bytes = client.get(client_url).send().await
                .map_err(|e| format!("Failed to download client: {e}"))?
                .bytes().await.map_err(|e| format!("Failed to read client bytes: {e}"))?;
            std::fs::write(&jar_path, &bytes)
                .map_err(|e| format!("Failed to write client jar: {e}"))?;
        }
    }
    Ok(())
}


// ── Tauri command ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn launch_instance(
    app_handle: tauri::AppHandle,
    instance_id: String,
    mc_version: String,
    mod_tool: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let instance_location = {
        let install_dir = state.settings.lock().unwrap().instance_install_dir.clone();
        find_instance_dir(Path::new(&install_dir), &instance_id)
            .ok_or_else(|| format!("Instance '{instance_id}' not found"))?
            .to_string_lossy()
            .into_owned()
    };
    let instance_meta: Option<InstanceMeta> = std::fs::read_to_string(
        PathBuf::from(&instance_location).join("instance.json")
    ).ok().and_then(|s| serde_json::from_str(&s).ok());

    let (java, ram_mb, window_width, window_height) = {
        let s = state.settings.lock().unwrap();
        let jre = instance_meta.as_ref().and_then(|m| if m.jre_path.is_empty() { None } else { Some(m.jre_path.clone()) })
            .unwrap_or_else(|| "javaw".to_string());
        let ram = instance_meta.as_ref().map(|m| if m.ram_mb > 0 { m.ram_mb } else { s.memory_mb }).unwrap_or(s.memory_mb);
        let width = instance_meta.as_ref().map(|m| if m.window_width > 0 { m.window_width } else { s.window_width }).unwrap_or(s.window_width);
        let height = instance_meta.as_ref().map(|m| if m.window_height > 0 { m.window_height } else { s.window_height }).unwrap_or(s.window_height);
        (jre, ram, width, height)
    };
    let versions: &Path = &state.versions_dir;
    let libs_dir: &Path = &state.libraries_dir;
    for dir in [versions, libs_dir] {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create directory {dir:?}: {e}"))?;
    }

    macro_rules! log {
        ($line:expr) => {
            let _ = app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: $line.to_string(), done: false, error: None,
            });
        };
    }
    macro_rules! done {
        ($line:expr) => {
            let _ = app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: $line.to_string(), done: true, error: None,
            });
        };
    }
    macro_rules! fail {
        ($msg:expr) => {{
            let m: String = $msg;
            let _ = app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: m.clone(), done: true, error: Some(m.clone()),
            });
            return Err(m);
        }};
    }

    log!("Resolving version JSON...");
    let version_id = find_version_id(&versions, &mc_version, &mod_tool);
    let manifest = match merged_manifest(&versions, &version_id, &state.http_client).await {
        Ok(m) => m,
        Err(e) => fail!(format!("Version JSON error: {e}")),
    };

    log!("Downloading libraries...");
    let warns = download_libraries(&manifest.libraries, &libs_dir, &state.http_client).await;
    for w in &warns { log!(format!("Warning: {w}")); }

    let natives_dir = versions.join(&version_id).join("natives");
    log!("Extracting natives...");
    if let Err(e) = extract_natives(&manifest.libraries, &libs_dir, &natives_dir) {
        log!(format!("Warning: natives: {e}"));
    }

    if manifest.main_class.is_empty() {
        fail!("mainClass not found in version JSON".to_string());
    }

    let classpath = build_classpath(&manifest.libraries, &libs_dir, &versions, &version_id);
    if classpath.is_empty() {
        fail!("Classpath is empty – libraries may not have downloaded correctly".to_string());
    }

    let game_dir = PathBuf::from(&instance_location);
    std::fs::create_dir_all(&game_dir)
        .map_err(|e| format!("Cannot create game dir: {e}"))?;
    let game_dir_str = game_dir.to_string_lossy().into_owned();
    let natives_str  = natives_dir.to_string_lossy().into_owned();
    let asset_index  = manifest.asset_index.as_ref().map_or(mc_version.as_str(), |a| a.id.as_str()).to_string();
    let assets_dir   = state.assets_dir.to_string_lossy().into_owned();

    let res_width  = if window_width  > 0 { window_width.to_string()  } else { String::new() };
    let res_height = if window_height > 0 { window_height.to_string() } else { String::new() };

    let vars = LaunchVars {
        natives_directory: &natives_str,
        classpath:         &classpath,
        launcher_name:     "yaminabe",
        launcher_version:  "0.1.0",
        auth_player_name:  "OfflinePlayer",
        version_name:      &mc_version,
        game_directory:    &game_dir_str,
        assets_root:       &assets_dir,
        assets_index_name: &asset_index,
        auth_uuid:         "00000000-0000-0000-0000-000000000000",
        auth_access_token: "0",
        user_type:         "offline",
        version_type:      "release",
        clientid:          "0",
        auth_xuid:         "0",
        resolution_width:  &res_width,
        resolution_height: &res_height,
    };

    log!(format!("Java: {java}"));
    log!(format!("Main class: {}", manifest.main_class));
    log!(format!("Game dir: {game_dir_str}"));
    log!("Starting process...");

    let mut cmd = tokio::process::Command::new(&java);

    if let Some(args) = &manifest.arguments {
        let default_jvm = collect_default_jvm(&args.default_user_jvm);
        if default_jvm.is_empty() {
            cmd.arg("-Xms512M");
            cmd.arg(format!("-Xmx{ram_mb}M"));
        } else {
            cmd.args(default_jvm);
        }
        cmd.args(process_args(&args.jvm, &vars));
        cmd.arg(&manifest.main_class);
        cmd.args(process_args(&args.game, &vars));
    } else {
        cmd.arg("-Xms512M");
        cmd.arg(format!("-Xmx{ram_mb}M"));
        cmd.args([
            format!("-Djava.library.path={natives_str}"),
            "-Dminecraft.launcher.brand=yaminabe".to_string(),
            "-cp".to_string(), classpath.clone(),
        ]);
        cmd.arg(&manifest.main_class);
        if let Some(s) = &manifest.minecraft_arguments {
            cmd.args(s.split_whitespace().map(|s| substitute_vars(s, &vars)));
        }
    }

    cmd.current_dir(&game_dir);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => fail!(format!("Failed to spawn Java: {e}")),
    };

    use tokio::io::{AsyncBufReadExt, BufReader};

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let app1 = app_handle.clone(); let id1 = instance_id.clone();
    let t1 = tokio::spawn(async move {
        let mut r = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = r.next_line().await {
            let _ = app1.emit("instance-log", LogLine { instance_id: id1.clone(), line, done: false, error: None });
        }
    });

    let app2 = app_handle.clone(); let id2 = instance_id.clone();
    let t2 = tokio::spawn(async move {
        let mut r = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = r.next_line().await {
            let _ = app2.emit("instance-log", LogLine { instance_id: id2.clone(), line: format!("[STDERR] {line}"), done: false, error: None });
        }
    });

    let status = child.wait().await.map_err(|e| e.to_string())?;
    let _ = tokio::join!(t1, t2);

    let exit_code = status.code().unwrap_or(-1);
    log!(format!("Process exited with code {exit_code}."));

    if !status.success() {
        let crash_dir = game_dir.join("crash-reports");
        if let Ok(entries) = std::fs::read_dir(&crash_dir) {
            let mut reports: Vec<std::fs::DirEntry> = entries
                .flatten()
                .filter(|e| e.path().extension().map_or(false, |x| x == "txt"))
                .collect();
            reports.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            if let Some(latest) = reports.last() {
                if let Ok(content) = std::fs::read_to_string(latest.path()) {
                    log!("─── Crash Report ──────────────────────────────");
                    for line in content.lines() {
                        log!(line.to_string());
                    }
                }
            }
        }
    }

    done!(format!("Done (exit code {exit_code})"));
    info!("Instance {instance_id} exited: {status}");
    Ok(())
}