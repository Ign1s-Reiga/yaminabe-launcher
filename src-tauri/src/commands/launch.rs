use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use log::info;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use yaminabe_launcher_shared::datatypes::{InstanceMeta, ModLoader};
use yaminabe_launcher_shared::error::Error;
use crate::{assets_dir, libraries_dir, runtimes_dir, versions_dir, AppState};
use crate::http_utils::sha1_hex;
use crate::install_task::forge_version_id;

use crate::commands::instance::find_instance_dir;
use crate::commands::java::download_java_runtime;

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
#[serde(rename_all = "camelCase")]
struct ClientManifest {
    #[serde(default)]
    main_class: String,
    #[serde(default)]
    arguments: Option<Arguments>,
    asset_index: Option<AssetIndex>,
    #[serde(default)]
    libraries: Vec<Library>,
    minecraft_arguments: Option<String>,
    inherits_from: Option<String>,
    java_version: Option<JavaVersion>,
}

#[derive(Deserialize)]
struct JavaVersion {
    component: String,
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
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    sha1: Option<String>,
}

#[derive(Deserialize)]
struct AssetIndexJson {
    objects: HashMap<String, AssetObject>,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
}

#[derive(Deserialize)]
struct Library {
    name: String,
    #[serde(default)]
    rules: Vec<ArgRule>,
    #[serde(default)]
    downloads: Option<LibraryDownloads>,
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

/// Compute the exact `versions/<id>` folder name for the given loader and
/// `mod_loader_version` recorded in the instance meta. Returns the version-id
/// string and the on-disk manifest path so the caller can verify it exists.
/// Does NOT scan the directory or fall back to other installed versions — if
/// the precise version isn't installed, the caller must surface a clear error
/// rather than launching with whatever happens to be present.
fn resolve_version_id(mc_version: &str, mod_loader: &ModLoader, mod_loader_version: Option<&str>) -> Result<String, Error> {
    let require_version = || mod_loader_version
        .ok_or_else(|| Error::Invalid(format!("Mod loader version required for {mod_loader}")));
    let id = match mod_loader {
        ModLoader::Vanilla => mc_version.to_string(),
        ModLoader::Fabric => format!("fabric-loader-{}-{mc_version}", require_version()?),
        ModLoader::Quilt => format!("quilt-loader-{}-{mc_version}", require_version()?),
        ModLoader::Forge => {
            let v = require_version()?;
            let build = v.strip_prefix("forge-").unwrap_or(v);
            forge_version_id(mc_version, build)
        }
        ModLoader::NeoForge => {
            let v = require_version()?;
            let build = v.strip_prefix("neoforge-").unwrap_or(v);
            format!("neoforge-{build}")
        }
    };
    Ok(id)
}

fn load_manifest(versions_dir: &Path, version_id: &str) -> Result<ClientManifest, Error> {
    let path = versions_dir.join(version_id).join(format!("{version_id}.json"));
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text)?)
}

/// Load and merge a version manifest, resolving `inheritsFrom` one level deep.
///
/// For non-array (single-value) fields, the child's value takes precedence and
/// the parent's is used only when the child's is absent. Array fields are
/// concatenated: the child's entries are kept, with the parent's entries
/// appended (deduplicated by name for libraries). The `arguments` object is a
/// composite whose inner arrays (`game`, `jvm`, `default_user_jvm`) are merged
/// parent-first so child entries can still override by appearing later.
fn merge_manifest(
    versions_dir: &Path,
    version_id: &str,
) -> Result<ClientManifest, Error> {
    let mut manifest = load_manifest(versions_dir, version_id)?;
    let Some(parent_id) = manifest.inherits_from.take() else {
        return Ok(manifest);
    };
    let parent = load_manifest(versions_dir, &parent_id)?;

    // Single-value fields: child wins; fall back to parent only when child is absent.
    manifest.asset_index = manifest.asset_index.or(parent.asset_index);
    manifest.minecraft_arguments = manifest.minecraft_arguments.or(parent.minecraft_arguments);
    manifest.java_version = manifest.java_version.or(parent.java_version);
    if manifest.main_class.is_empty() {
        manifest.main_class = parent.main_class;
    }

    // Composite arguments: concatenate each inner array parent-first.
    manifest.arguments = match (manifest.arguments.take(), parent.arguments) {
        (Some(mut child), Some(parent)) => {
            let mut game = parent.game;
            game.extend(child.game.drain(..));
            child.game = game;
            let mut jvm = parent.jvm;
            jvm.extend(child.jvm.drain(..));
            child.jvm = jvm;
            let mut default_user_jvm = parent.default_user_jvm;
            default_user_jvm.extend(child.default_user_jvm.drain(..));
            child.default_user_jvm = default_user_jvm;
            Some(child)
        }
        (child, parent) => child.or(parent),
    };

    // Array field: keep child entries, append parent entries whose names are new.
    let child_names: HashSet<String> = manifest.libraries.iter().map(|l| l.name.clone()).collect();
    for lib in parent.libraries {
        if !child_names.contains(&lib.name) {
            manifest.libraries.push(lib);
        }
    }
    Ok(manifest)
}

fn maven_to_path(name: &str) -> Option<String> {
    let (group, artifact, version, classifier) = maven_parts(name)?;
    let group = group.replace('.', "/");
    let suffix = match classifier {
        Some(cls) => format!("{artifact}-{version}-{cls}.jar"),
        None => format!("{artifact}-{version}.jar"),
    };
    Some(format!("{group}/{artifact}/{version}/{suffix}"))
}

fn maven_parts(name: &str) -> Option<(&str, &str, &str, Option<&str>)> {
    let mut parts = name.splitn(4, ':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    let classifier = parts.next();
    Some((group, artifact, version, classifier))
}

fn extract_natives(
    libraries: &[Library],
    libraries_dir: &Path,
    natives_root: &Path,
) -> Result<Vec<PathBuf>, Error> {
    std::fs::create_dir_all(natives_root)?;
    let mut native_dirs = Vec::new();
    for lib in libraries {
        if !eval_rules(&lib.rules) { continue; }
        let key = lib.natives.get("windows").map(|s| s.replace("${arch}", "64"));
        let Some(key) = key else { continue; };
        let Some((group, artifact_name, version, _)) = maven_parts(&lib.name) else { continue; };
        let Some(classifiers) = lib.downloads.as_ref().and_then(|d| d.classifiers.as_ref()) else { continue; };
        let Some(artifact) = classifiers.get(&key) else { continue; };
        let Some(path) = &artifact.path else { continue; };
        let jar_path = libraries_dir.join(path);
        if !jar_path.exists() { continue; }
        let Ok(file)    = std::fs::File::open(&jar_path) else { continue; };
        let Ok(mut zip) = zip::ZipArchive::new(file) else { continue; };
        let native_dir = natives_root
            .join(group.replace('.', "/"))
            .join(artifact_name)
            .join(version);
        std::fs::create_dir_all(&native_dir)?;
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else { continue; };
            let name = entry.name().to_string();
            if !name.ends_with(".dll") && !name.ends_with(".so") && !name.ends_with(".dylib") { continue; }
            let Some(file_name) = Path::new(&name).file_name() else { continue; };
            let dest = native_dir.join(file_name);
            if dest.exists() { continue; }
            let Ok(mut f) = std::fs::File::create(&dest) else { continue; };
            std::io::copy(&mut entry, &mut f).ok();
        }
        native_dirs.push(native_dir);
    }
    Ok(native_dirs)
}

/// Strip the version segment from a maven coordinate so duplicates with
/// different versions collapse. Classifier (if present) is preserved so e.g.
/// `forge:…:universal` and `forge:…:client` remain distinct entries.
fn library_path(lib: &Library, libraries_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()).and_then(|a| a.path.as_deref()) {
        Some(libraries_dir.join(path))
    } else {
        maven_to_path(&lib.name).map(|rel| libraries_dir.join(rel))
    }
}

fn jar_contains_class(jar_path: &Path, main_class: &str) -> bool {
    let class_path = format!("{}.class", main_class.replace('.', "/"));
    let Ok(file) = std::fs::File::open(jar_path) else { return false; };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return false; };
    zip.by_name(&class_path).is_ok()
}

fn find_main_class_jar(
    libraries: &[Library],
    libraries_dir: &Path,
    main_class: &str,
) -> Option<PathBuf> {
    let mut seen: HashSet<String> = HashSet::new();
    for lib in libraries {
        if !eval_rules(&lib.rules) { continue; }
        if !lib.natives.is_empty() { continue; }
        let key = version_agnostic_name(&lib.name);
        if !seen.insert(key) { continue; }
        let Some(path) = library_path(lib, libraries_dir) else { continue; };
        if path.exists() && jar_contains_class(&path, main_class) {
            return Some(path);
        }
    }
    None
}

fn version_agnostic_name(maven_coord: &str) -> String {
    let parts: Vec<&str> = maven_coord.splitn(4, ':').collect();
    let group = parts.first().copied().unwrap_or("");
    let artifact = parts.get(1).copied().unwrap_or("");
    match parts.get(3) {
        Some(classifier) => format!("{group}:{artifact}:{classifier}"),
        None => format!("{group}:{artifact}"),
    }
}

fn build_classpath(
    libraries: &[Library],
    libraries_dir: &Path,
    versions_dir: &Path,
    mc_version: &str,
    version_id: &str,
) -> (String, Vec<String>) {
    // Two filters before each library becomes a classpath entry:
    //   1. Skip old-style natives libs (non-empty `natives` map). Their
    //      classifier jars are unpacked by `extract_natives` and don't belong
    //      on classpath.
    //   2. Dedupe by `group:artifact[:classifier]`. Merged manifests (esp.
    //      newer MC + loader combos) may carry multiple versions of the same
    //      artifact; keep the first occurrence. In our merge order (child
    //      first) that's the version the loader explicitly asked for.
    // Returns (classpath, missing) — missing lists manifest libraries whose
    // resolved on-disk path didn't exist, so the caller can surface them
    // instead of failing later with an opaque ClassNotFoundException.
    let mut seen: HashSet<String> = HashSet::new();
    let mut paths: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    // Per-version primary jar at `versions/<id>/<id>.jar`. This is the patched
    // client jar for Forge/NeoForge and the vanilla client jar for Vanilla.
    // Any manifest library entry pointing to the same artifact (e.g.
    // `net.minecraftforge:forge:<ver>` for V1 Forge) intentionally resolves to
    // a path under `libraries/` that no longer holds the file — we suppress
    // the corresponding `missing` warning below.
    let primary_jar = versions_dir.join(version_id).join(format!("{version_id}.jar"));
    if primary_jar.exists() {
        paths.push(primary_jar.to_string_lossy().into_owned());
    }
    for lib in libraries {
        if !eval_rules(&lib.rules) { continue; }
        if !lib.natives.is_empty() { continue; }
        let key = version_agnostic_name(&lib.name);
        if !seen.insert(key) { continue; }
        if let Some(p) = library_path(lib, libraries_dir) {
            if p.exists() {
                paths.push(p.to_string_lossy().into_owned());
            } else if !is_primary_jar_lib(&lib.name) {
                missing.push(format!("{} (expected at {})", lib.name, p.display()));
            }
        }
    }
    // Vanilla client jar is also required for non-Vanilla loaders that don't
    // bundle the unmodified vanilla classes into their patched jar (Fabric,
    // Quilt, and pre-1.13 Forge). For Vanilla itself this is the primary jar
    // already appended above; skip the duplicate.
    if version_id != mc_version {
        let vanilla_jar = versions_dir.join(mc_version).join(format!("{mc_version}.jar"));
        if vanilla_jar.exists() { paths.push(vanilla_jar.to_string_lossy().into_owned()); }
    }
    (paths.join(";"), missing)
}

/// Library entries whose artifact IS the primary (patched client) jar that
/// now lives in `versions/<id>/<id>.jar`. The library's `path` still points
/// into `libraries/` but no file is written there — keep the entry quiet.
fn is_primary_jar_lib(maven_name: &str) -> bool {
    let mut parts = maven_name.splitn(3, ':');
    matches!(
        (parts.next(), parts.next()),
        (Some("net.minecraftforge"), Some("forge")) | (Some("net.neoforged"), Some("neoforge"))
    )
}

struct LaunchVars<'a> {
    natives_directory: &'a str,
    classpath: &'a str,
    classpath_separator: &'a str,
    library_directory: &'a str,
    launcher_name: &'a str,
    launcher_version: &'a str,
    auth_player_name: &'a str,
    version_name: &'a str,
    game_directory: &'a str,
    assets_root: &'a str,
    assets_index_name: &'a str,
    auth_uuid: &'a str,
    auth_access_token: &'a str,
    user_type: &'a str,
    version_type: &'a str,
    clientid: &'a str,
    auth_xuid: &'a str,
    resolution_width: &'a str,
    resolution_height: &'a str,
}

fn substitute_vars(s: &str, v: &LaunchVars) -> String {
    s.replace("${natives_directory}", v.natives_directory)
     .replace("${classpath_separator}", v.classpath_separator)
     .replace("${classpath}", v.classpath)
     .replace("${library_directory}", v.library_directory)
     .replace("${launcher_name}", v.launcher_name)
     .replace("${launcher_version}", v.launcher_version)
     .replace("${auth_player_name}", v.auth_player_name)
     .replace("${version_name}", v.version_name)
     .replace("${game_directory}", v.game_directory)
     .replace("${assets_root}", v.assets_root)
     .replace("${assets_index_name}", v.assets_index_name)
     .replace("${auth_uuid}", v.auth_uuid)
     .replace("${auth_access_token}", v.auth_access_token)
     .replace("${user_type}", v.user_type)
     .replace("${version_type}", v.version_type)
     .replace("${clientid}", v.clientid)
     .replace("${auth_xuid}", v.auth_xuid)
     .replace("${resolution_width}", v.resolution_width)
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

/// Stable dedup over an iterator of strings — keeps the first occurrence of
/// each value, preserving the original order of the remainder.
fn dedup_preserve_order(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    items.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

fn extend_from_arg_value(out: &mut Vec<String>, value: &ArgValue, mut map: impl FnMut(&str) -> String) {
    match value {
        ArgValue::One(s) => out.push(map(s)),
        ArgValue::Many(v) => out.extend(v.iter().map(|s| map(s))),
    }
}

fn collect_default_jvm(items: &[DefaultJvmItem]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if item.rules.iter().any(|r| r.features.is_some()) { continue; }
        if !eval_rules(&item.rules) { continue; }
        extend_from_arg_value(&mut out, &item.value, |s| s.to_string());
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
                        extend_from_arg_value(&mut out, value, |s| substitute_vars(s, vars));
                    }
                    continue;
                }
                if rules.iter().any(|r| r.features.is_some()) { continue; }
                if !eval_rules(rules) { continue; }
                extend_from_arg_value(&mut out, value, |s| substitute_vars(s, vars));
            }
        }
    }
    out
}

/// Fetch `url`, verify its SHA1 matches `expected_sha1`, and return the bytes.
async fn fetch_and_verify(
    client: &reqwest::Client,
    url: &str,
    expected_sha1: &str,
    resource: &str,
) -> Result<Vec<u8>, Error> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), url.to_string()));
    }
    let bytes = resp.bytes().await.map_err(Error::InvalidResponse)?.to_vec();
    let hex = sha1_hex(&bytes);
    if hex != expected_sha1 {
        return Err(Error::ChecksumMismatch {
            resource: resource.to_string(),
            sha1: expected_sha1.to_string(),
            hex,
        });
    }
    Ok(bytes)
}

async fn download_assets(
    assets_dir: &Path,
    asset_index: &AssetIndex,
    client: &reqwest::Client,
    mut log_progress: impl FnMut(String),
) -> Result<(), Error> {
    let indexes_dir = assets_dir.join("indexes");
    std::fs::create_dir_all(&indexes_dir)?;
    let asset_index_file_name = format!("{}.json", asset_index.id);
    let index_path = indexes_dir.join(&asset_index_file_name);

    let index_bytes = if index_path.exists() {
        log_progress(format!(
            "Assets: using cached asset index {} (file: {}).",
            asset_index.id, asset_index_file_name
        ));
        std::fs::read(&index_path)?
    } else {
        log_progress(format!(
            "Assets: downloading asset index {} (file: {}).",
            asset_index.id, asset_index_file_name
        ));
        let url = asset_index.url.as_ref()
            .ok_or_else(|| Error::Invalid(format!("asset index {} missing url", asset_index.id)))?;
        let sha1 = asset_index.sha1.as_ref()
            .ok_or_else(|| Error::Invalid(format!("asset index {} missing sha1", asset_index.id)))?;
        let bytes = fetch_and_verify(client, url, sha1, &format!("asset index {}", asset_index.id)).await?;
        std::fs::write(&index_path, &bytes)?;
        bytes
    };

    let parsed: AssetIndexJson = serde_json::from_slice(&index_bytes)?;
    let objects_dir = assets_dir.join("objects");
    let total_assets = parsed.objects.len();
    let mut checked_assets = 0usize;
    let mut cached_assets = 0usize;
    let mut downloaded_assets = 0usize;
    const ASSET_PROGRESS_INTERVAL: usize = 100;

    log_progress(format!("Verifying {total_assets} indexed asset files..."));

    for (path, object) in &parsed.objects {
        if object.hash.len() < 2 {
            return Err(Error::Invalid(format!("asset {path} has invalid hash {}", object.hash)));
        }
        let prefix = &object.hash[..2];
        let dest_dir = objects_dir.join(prefix);
        let dest = dest_dir.join(&object.hash);
        let needs_download = if dest.exists() {
            match std::fs::read(&dest) {
                Ok(bytes) => {
                    let hex = sha1_hex(&bytes);
                    if hex == object.hash {
                        cached_assets += 1;
                        false
                    } else {
                        log_progress(format!(
                            "Failed to verify '{path}' (Hash mismatch: expected {}, got {}).",
                            object.hash, hex
                        ));
                        true
                    }
                }
                Err(e) => {
                    log_progress(format!(
                        "Failed to read cached asset '{path}' ({e})."
                    ));
                    true
                }
            }
        } else {
            true
        };
        if needs_download {
            let url = format!("https://resources.download.minecraft.net/{prefix}/{}", object.hash);
            let bytes = match fetch_and_verify(client, &url, &object.hash, &format!("asset {path}")).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    log_progress(format!("Failed to download '{path}' ({e})."));
                    return Err(e);
                }
            };
            std::fs::create_dir_all(&dest_dir)?;
            std::fs::write(&dest, &bytes)?;
            downloaded_assets += 1;
        }

        checked_assets += 1;
        if checked_assets == total_assets || checked_assets % ASSET_PROGRESS_INTERVAL == 0 {
            let percent = if total_assets == 0 { 100 } else { checked_assets * 100 / total_assets };
            log_progress(format!(
                "Downloading Assets: {percent}% ({checked_assets}/{total_assets}); {downloaded_assets} downloaded, {cached_assets} verified."
            ));
        }
    }
    if total_assets == 0 {
        log_progress("Asset verification complete. Asset index contained no files.".to_string());
    } else if downloaded_assets == 0 {
        log_progress("Asset verification complete. All files matched.".to_string());
    } else {
        log_progress(format!(
            "Asset synchronization complete. {downloaded_assets} downloaded, {cached_assets} verified."
        ));
    }
    Ok(())
}


// ── Tauri command ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn launch_instance(
    app_handle: tauri::AppHandle,
    instance_id: String,
    mc_version: String,
    mod_loader: ModLoader,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let instance_location = {
        let install_dir = state.settings.lock().unwrap().instance_install_dir.clone();
        find_instance_dir(Path::new(&install_dir), &instance_id)?
            .to_string_lossy()
            .into_owned()
    };
    let instance_meta: Option<InstanceMeta> = std::fs::read_to_string(
        PathBuf::from(&instance_location).join("instance.json")
    ).ok().and_then(|s| serde_json::from_str(&s).ok());

    let (instance_jre_path, ram_mb, window_width, window_height) = {
        let s = state.settings.lock().unwrap();
        let jre = instance_meta.as_ref()
            .and_then(|m| if m.jre_path.is_empty() { None } else { Some(m.jre_path.clone()) });
        let ram = instance_meta.as_ref().map(|m| if m.ram_mb > 0 { m.ram_mb } else { s.memory_mb }).unwrap_or(s.memory_mb);
        let width = instance_meta.as_ref().map(|m| if m.window_width > 0 { m.window_width } else { s.window_width }).unwrap_or(s.window_width);
        let height = instance_meta.as_ref().map(|m| if m.window_height > 0 { m.window_height } else { s.window_height }).unwrap_or(s.window_height);
        (jre, ram, width, height)
    };
    for dir in [versions_dir(), libraries_dir()] {
        std::fs::create_dir_all(dir)?;
    }

    macro_rules! log {
        ($line:expr) => {
            app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: $line.to_string(), done: false, error: None,
            }).ok();
        };
    }
    macro_rules! done {
        ($line:expr) => {
            app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: $line.to_string(), done: true, error: None,
            }).ok();
        };
    }
    macro_rules! fail {
        ($e:expr) => {{
            let msg = $e.to_string();
            app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: msg.clone(), done: true, error: Some(msg),
            }).ok();
            return Err($e);
        }};
    }

    log!("Resolving version JSON...");
    let mod_loader_version = instance_meta.as_ref()
        .and_then(|m| m.mod_loader_version.clone());
    let version_id = match resolve_version_id(&mc_version, &mod_loader, mod_loader_version.as_deref()) {
        Ok(id) => id,
        Err(e) => fail!(e),
    };
    let manifest_path = versions_dir().join(&version_id).join(format!("{version_id}.json"));
    if !manifest_path.exists() {
        fail!(Error::Invalid("The required versions for launch could not be found.".to_string()));
    }
    let child_manifest = match load_manifest(versions_dir(), &version_id) {
        Ok(m) => m,
        Err(e) => fail!(e),
    };
    let has_parent_manifest = child_manifest.inherits_from.is_some();
    let manifest = match merge_manifest(versions_dir(), &version_id) {
        Ok(m) => m,
        Err(e) => fail!(e),
    };

    // jrePath in instance.json overrides everything; otherwise use the JRE
    // recommended by client.json (javaVersion.component → runtimes/<component>).
    // If the Recommended runtime is missing on disk, fetch it on demand —
    // launching with the wrong major version is a silent ClassCastException
    // (LaunchWrapper needs Java 8) so falling back to system `java` is unsafe.
    //
    // We pick `java.exe` (not `javaw.exe`): javaw is a windowless launcher that
    // suppresses stdout/stderr — including JVM startup errors — which makes
    // crashes unobservable. With `CREATE_NO_WINDOW` set on spawn we still avoid
    // the console pop-up.
    let java = if let Some(custom) = instance_jre_path {
        custom
    } else if let Some(component) = manifest.java_version.as_ref().map(|jv| jv.component.clone()) {
        let java_exe = runtimes_dir().join(&component).join("bin").join("java.exe");
        if !java_exe.exists() {
            log!(format!("Recommended runtime '{component}' missing; downloading..."));
            if let Err(e) = download_java_runtime(&component, &state.http_client).await {
                fail!(Error::Invalid(format!("failed to download recommended JRE '{component}': {e}")));
            }
        }
        if !java_exe.exists() {
            fail!(Error::Invalid(format!("java.exe not found after downloading runtime '{component}'")));
        }
        java_exe.to_string_lossy().into_owned()
    } else {
        "java".to_string()
    };

    let asset_index = match manifest.asset_index.as_ref() {
        Some(idx) => idx,
        None => fail!(Error::Invalid("assetIndex not found in version JSON".into())),
    };
    let asset_index_file_name = format!("{}.json", asset_index.id);
    log!(format!(
        "[AssetManager/INFO]: Loading asset index: {} (id: {})",
        asset_index_file_name, asset_index.id
    ));
    if let Err(e) = download_assets(assets_dir(), asset_index, &state.http_client, |line| {
        log!(line);
    }).await {
        fail!(e);
    }

    let natives_dir = libraries_dir().join("natives");
    log!("Extracting natives...");
    let native_dirs = match extract_natives(&manifest.libraries, libraries_dir(), &natives_dir) {
        Ok(dirs) => dedup_preserve_order(dirs.into_iter().map(|p| p.to_string_lossy().into_owned())),
        Err(e) => {
            log!(format!("Warning: natives: {e}"));
            Vec::new()
        }
    };

    if manifest.main_class.is_empty() {
        fail!(Error::Invalid("mainClass not found in version JSON".into()));
    }

    let entry_libraries = if has_parent_manifest {
        &child_manifest.libraries
    } else {
        &manifest.libraries
    };
    let entry_jar = find_main_class_jar(entry_libraries, libraries_dir(), &manifest.main_class)
        .or_else(|| {
            if has_parent_manifest {
                return None;
            }
            let vanilla_jar = versions_dir().join(&mc_version).join(format!("{mc_version}.jar"));
            (vanilla_jar.exists() && jar_contains_class(&vanilla_jar, &manifest.main_class)).then_some(vanilla_jar)
        });
    let Some(entry_jar) = entry_jar else {
        fail!(Error::Invalid(format!(
            "could not resolve entry jar containing mainClass {}",
            manifest.main_class
        )));
    };
    log!(format!("Entry jar: {}", entry_jar.display()));

    let (classpath, missing_libs) = build_classpath(&manifest.libraries, libraries_dir(), versions_dir(), &mc_version, &version_id);
    for m in &missing_libs {
        log!(format!("Warning: manifest library missing on disk: {m}"));
    }
    if classpath.is_empty() {
        fail!(Error::Invalid("classpath is empty — libraries may not have downloaded correctly".into()));
    }

    let game_dir = PathBuf::from(&instance_location);
    std::fs::create_dir_all(&game_dir)?;
    let game_dir_str = game_dir.to_string_lossy().into_owned();
    let natives_str = if native_dirs.is_empty() {
        natives_dir.to_string_lossy().into_owned()
    } else {
        native_dirs.join(if cfg!(windows) { ";" } else { ":" })
    };
    let asset_index_name = asset_index.id.clone();
    let assets_dir = assets_dir().to_string_lossy().into_owned();

    let res_width = if window_width > 0 { window_width.to_string() } else { String::new() };
    let res_height = if window_height > 0 { window_height.to_string() } else { String::new() };
    let libraries_dir_str = libraries_dir().to_string_lossy().into_owned();
    let classpath_separator = if cfg!(windows) { ";" } else { ":" };

    let vars = LaunchVars {
        natives_directory: &natives_str,
        classpath: &classpath,
        classpath_separator,
        library_directory: &libraries_dir_str,
        launcher_name: "yaminabe",
        launcher_version: "0.1.0",
        auth_player_name: "OfflinePlayer",
        version_name: &mc_version,
        game_directory: &game_dir_str,
        assets_root: &assets_dir,
        assets_index_name: &asset_index_name,
        auth_uuid: "00000000-0000-0000-0000-000000000000",
        auth_access_token: "0",
        user_type: "offline",
        version_type: "release",
        clientid: "0",
        auth_xuid: "0",
        resolution_width: &res_width,
        resolution_height: &res_height,
    };

    log!(format!("Java: {java}"));
    log!(format!("Main class: {}", manifest.main_class));
    log!(format!("Game dir: {game_dir_str}"));

    // Build the full argument list up front so we can log it verbatim before
    // spawning — useful for diagnosing crashes where Java exits before printing.
    // Layout: [JVM args] [main-class | -jar <app.jar>] [game args].
    let mut launch_args: Vec<String> = Vec::new();
    if let Some(args) = &manifest.arguments {
        let mut jvm_section: Vec<String> = Vec::new();
        let default_jvm = collect_default_jvm(&args.default_user_jvm);
        if default_jvm.is_empty() {
            jvm_section.push("-Xms512M".to_string());
            jvm_section.push(format!("-Xmx{ram_mb}M"));
        } else {
            jvm_section.extend(default_jvm);
        }
        jvm_section.extend(process_args(&args.jvm, &vars));
        launch_args.extend(jvm_section);

        // All loaders (including Forge/NeoForge) launch via the manifest's
        // mainClass with a populated classpath. For Forge that main class is
        // the bootstrap launcher (e.g. cpw.mods.bootstraplauncher.BootstrapLauncher),
        // which relies on `${classpath}` substitutions and module-path JVM args
        // declared in the manifest's `arguments.jvm`. Using `-jar` here would
        // make the JVM ignore `-cp` / `${classpath}` entirely and break the
        // launch — the patched forge client jar is just one classpath entry
        // among many, not a self-contained launcher.
        // `entry_jar` was resolved earlier purely as an existence sanity check.
        launch_args.push(manifest.main_class.clone());
        launch_args.extend(process_args(&args.game, &vars));
    } else {
        launch_args.push("-Xms512M".to_string());
        launch_args.push(format!("-Xmx{ram_mb}M"));
        launch_args.push(format!("-Djava.library.path={natives_str}"));
        launch_args.push("-Dminecraft.launcher.brand=yaminabe".to_string());
        launch_args.push("-cp".to_string());
        launch_args.push(classpath.clone());
        launch_args.push(manifest.main_class.clone());
        if let Some(s) = &manifest.minecraft_arguments {
            launch_args.extend(s.split_whitespace().map(|s| substitute_vars(s, &vars)));
        }
    }

    log!("Launch command:");
    log!(format!("  {java}"));
    // `{:?}` so hidden whitespace / control chars in the substituted strings
    // surface as escapes (`\r`, `\u{a0}`, …) rather than rendering invisibly.
    for a in &launch_args {
        log!(format!("    {a:?}"));
    }
    log!("Starting process...");

    let mut cmd = tokio::process::Command::new(&java);
    cmd.args(&launch_args);
    cmd.current_dir(&game_dir);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    // CREATE_NO_WINDOW = 0x08000000 — suppress the console window that `java.exe`
    // (a console subsystem binary) would otherwise pop up when launched from a
    // GUI parent. Redirected stdio still works, unlike javaw.
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => fail!(Error::ChildProcess(format!("spawning Java process: {e}"))),
    };
    if let Some(pid) = child.id() {
        state.running_children.lock().unwrap().insert(instance_id.clone(), pid);
    }

    // Read bytes (not lines) so we can decode lossily. Java emits stderr in the
    // system codepage on Windows (e.g. CP932 on Japanese locales), and
    // `BufReader::lines()` aborts the stream as soon as it sees a non-UTF-8
    // byte — which is exactly when we most want the output (early JVM errors).
    async fn drain(
        mut reader: impl tokio::io::AsyncRead + Unpin,
        app: tauri::AppHandle,
        instance_id: String,
        is_stderr: bool,
    ) {
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 4096];
        let mut leftover: Vec<u8> = Vec::new();
        let emit = |app: &tauri::AppHandle, id: &str, bytes: &[u8]| {
            let raw = String::from_utf8_lossy(bytes);
            let trimmed = raw.trim_end_matches(|c| c == '\r' || c == '\n');
            let line = if is_stderr { format!("[STDERR] {trimmed}") } else { trimmed.to_string() };
            app.emit("instance-log", LogLine {
                instance_id: id.to_string(), line, done: false, error: None,
            }).ok();
        };
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    leftover.extend_from_slice(&buf[..n]);
                    while let Some(pos) = leftover.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = leftover.drain(..=pos).collect();
                        emit(&app, &instance_id, &line_bytes);
                    }
                }
            }
        }
        if !leftover.is_empty() {
            emit(&app, &instance_id, &leftover);
        }
    }

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let t1 = tokio::spawn(drain(stdout, app_handle.clone(), instance_id.clone(), false));
    let t2 = tokio::spawn(drain(stderr, app_handle.clone(), instance_id.clone(), true));

    let status = child.wait().await?;
    state.running_children.lock().unwrap().remove(&instance_id);
    t1.await.ok();
    t2.await.ok();

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

/// Terminate the Java process tree for a running instance. We shell out to
/// `taskkill /F /T /PID <pid>` rather than calling `tokio::Child::kill` because
/// the `Child` is owned by `launch_instance` (which holds it across an `await`
/// on `wait()`) and forwarding ownership would require restructuring the entire
/// drain/wait pipeline. `/T` walks the descendant tree so child JVMs/launchers
/// spawned by the game (e.g. crash reporters) get cleaned up too.
#[tauri::command]
pub async fn kill_instance(
    instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), Error> {
    let pid = state.running_children.lock().unwrap().get(&instance_id).copied();
    let Some(pid) = pid else {
        return Err(Error::NotExists(format!("running instance '{instance_id}'")));
    };

    #[cfg(windows)]
    {
        let status = tokio::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x08000000)
            .status().await
            .map_err(|e| Error::ChildProcess(format!("invoking taskkill for pid {pid}: {e}")))?;
        if !status.success() {
            return Err(Error::ChildProcess(format!("taskkill for pid {pid} exited with {status}")));
        }
    }
    #[cfg(not(windows))]
    {
        let status = tokio::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status().await
            .map_err(|e| Error::ChildProcess(format!("invoking kill for pid {pid}: {e}")))?;
        if !status.success() {
            return Err(Error::ChildProcess(format!("kill for pid {pid} exited with {status}")));
        }
    }
    Ok(())
}
