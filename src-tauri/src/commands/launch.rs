use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use log::info;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tauri::{Emitter, State};
use yaminabe_launcher_shared::datatypes::{InstanceMeta, ModLoader};
use yaminabe_launcher_shared::error::Error;
use crate::{assets_dir, libraries_dir, runtimes_dir, versions_dir, AppState};

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

fn find_version_id(versions_dir: &Path, mc_version: &str, mod_loader: &ModLoader) -> String {
    if matches!(mod_loader, ModLoader::Vanilla) {
        return mc_version.to_string();
    }
    let prefix = match mod_loader {
        ModLoader::Fabric => "fabric-loader-".to_string(),
        ModLoader::Quilt => "quilt-loader-".to_string(),
        ModLoader::Forge => format!("{mc_version}-forge-"),
        ModLoader::NeoForge => "neoforge-".to_string(),
        _ => return mc_version.to_string(),
    };
    let Ok(entries) = std::fs::read_dir(versions_dir) else {
        return mc_version.to_string();
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) {
            let loader_needs_mc = matches!(mod_loader, ModLoader::Fabric | ModLoader::Quilt);
            if !loader_needs_mc || name.ends_with(mc_version) {
                return name;
            }
        }
    }
    mc_version.to_string()
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

fn extract_natives(
    libraries: &[Library],
    libraries_dir: &Path,
    natives_dir: &Path,
) -> Result<(), Error> {
    std::fs::create_dir_all(natives_dir)?;
    for lib in libraries {
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
            std::io::copy(&mut entry, &mut f).ok();
        }
    }
    Ok(())
}

/// Build the path to a mod loader's patched client jar, based on the maven
/// coordinate (`{group}:{artifact}:{version}`) of its entry in the merged
/// libraries list. Layout: `libraries/{group as dirs}/{artifact}/{version}/{artifact}-{version}-client.jar`.
fn loader_patched_client_jar(
    libraries: &[Library],
    libraries_dir: &Path,
    group: &str,
    artifact: &str,
) -> Option<PathBuf> {
    let prefix = format!("{group}:{artifact}:");
    let version = libraries.iter()
        .find_map(|l| l.name.strip_prefix(&prefix).and_then(|rest| rest.split(':').next()))?;
    let mut path = libraries_dir.to_path_buf();
    for part in group.split('.') {
        path = path.join(part);
    }
    Some(path.join(artifact).join(version).join(format!("{artifact}-{version}-client.jar")))
}

/// Resolve the on-disk path of a library identified by `{group}:{artifact}`
/// prefix, using the `downloads.artifact.path` recorded in the manifest entry.
/// Unlike `loader_patched_client_jar` this works for ordinary library jars
/// (no `:client` classifier appended), which is what modern NeoForge needs
/// for the FML loader jar that hosts `mainClass`.
fn library_jar_path(
    libraries: &[Library],
    libraries_dir: &Path,
    group_artifact_prefix: &str,
) -> Option<PathBuf> {
    let lib = libraries.iter().find(|l| l.name.starts_with(group_artifact_prefix))?;
    let path = lib.downloads.as_ref()?.artifact.as_ref()?.path.as_deref()?;
    Some(libraries_dir.join(path))
}

/// Resolve the application jar that should be appended to the classpath.
///
/// Confirmed cases:
/// * Vanilla → `versions/{mc_version}/{mc_version}.jar`.
/// * Fabric → vanilla parent jar; the loader is supplied via the regular
///   `libraries` entries (see `build_classpath`).
/// * Forge (26.1.2+) → patched client jar under `libraries/net/minecraftforge/forge/…-client.jar`.
///
/// Unconfirmed but assumed to follow the same shape as their counterparts:
/// Quilt mirrors Fabric, NeoForge mirrors Forge.
fn entry_jar_path(
    libraries: &[Library],
    libraries_dir: &Path,
    versions_dir: &Path,
    mc_version: &str,
    mod_loader: &ModLoader,
) -> Option<PathBuf> {
    match mod_loader {
        ModLoader::Vanilla | ModLoader::Fabric | ModLoader::Quilt => {
            Some(versions_dir.join(mc_version).join(format!("{mc_version}.jar")))
        }
        ModLoader::Forge => {
            loader_patched_client_jar(libraries, libraries_dir, "net.minecraftforge", "forge")
        }
        ModLoader::NeoForge => {
            // NeoForge does not emit a patched `:client` classifier jar; the
            // `mainClass` (e.g. `net.neoforged.fml.startup.Client`) lives in
            // `net.neoforged.fancymodloader:loader:…`.
            library_jar_path(libraries, libraries_dir, "net.neoforged.fancymodloader:loader:")
        }
    }
}

/// Strip the version segment from a maven coordinate so duplicates with
/// different versions collapse. Classifier (if present) is preserved so e.g.
/// `forge:…:universal` and `forge:…:client` remain distinct entries.
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
    mod_loader: &ModLoader,
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
    for lib in libraries {
        if !lib.natives.is_empty() { continue; }
        let key = version_agnostic_name(&lib.name);
        if !seen.insert(key) { continue; }
        let p = if let Some(path) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()).and_then(|a| a.path.as_deref()) {
            Some(libraries_dir.join(path))
        } else if let Some(rel) = maven_to_path(&lib.name) {
            Some(libraries_dir.join(rel))
        } else { None };
        if let Some(p) = p {
            if p.exists() {
                paths.push(p.to_string_lossy().into_owned());
            } else {
                missing.push(format!("{} (expected at {})", lib.name, p.display()));
            }
        }
    }
    // For mainClass-based loaders (Vanilla/Fabric/Quilt/NeoForge), append the
    // per-version client jar that lives under `versions/`. Forge is the
    // exception: its patched client jar lives under `libraries/` and was
    // already picked up above as a manifest library entry, so the vanilla jar
    // is not used at runtime.
    if !matches!(mod_loader, ModLoader::Forge) {
        let jar = versions_dir.join(mc_version).join(format!("{mc_version}.jar"));
        if jar.exists() { paths.push(jar.to_string_lossy().into_owned()); }
    }
    (paths.join(";"), missing)
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
    let hex = Sha1::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
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
) -> Result<(), Error> {
    let indexes_dir = assets_dir.join("indexes");
    std::fs::create_dir_all(&indexes_dir)?;
    let index_path = indexes_dir.join(format!("{}.json", asset_index.id));

    let index_bytes = if index_path.exists() {
        std::fs::read(&index_path)?
    } else {
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

    for (path, object) in &parsed.objects {
        if object.hash.len() < 2 {
            return Err(Error::Invalid(format!("asset {path} has invalid hash {}", object.hash)));
        }
        let prefix = &object.hash[..2];
        let dest_dir = objects_dir.join(prefix);
        let dest = dest_dir.join(&object.hash);
        if dest.exists() { continue; }
        let url = format!("https://resources.download.minecraft.net/{prefix}/{}", object.hash);
        let bytes = fetch_and_verify(client, &url, &object.hash, &format!("asset {path}")).await?;
        std::fs::create_dir_all(&dest_dir)?;
        std::fs::write(&dest, &bytes)?;
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
    let version_id = find_version_id(versions_dir(), &mc_version, &mod_loader);
    let manifest = match merge_manifest(versions_dir(), &version_id) {
        Ok(m) => m,
        Err(e) => fail!(e),
    };

    // jrePath in instance.json overrides everything; otherwise use the JRE
    // recommended by client.json (javaVersion.component → runtimes/<component>);
    // fall back to system java if neither is usable.
    //
    // We pick `java.exe` (not `javaw.exe`): javaw is a windowless launcher that
    // suppresses stdout/stderr — including JVM startup errors — which makes
    // crashes unobservable. With `CREATE_NO_WINDOW` set on spawn we still avoid
    // the console pop-up.
    let java = instance_jre_path.or_else(|| {
        let component = &manifest.java_version.as_ref()?.component;
        let path = runtimes_dir().join(component).join("bin").join("java.exe");
        path.exists().then(|| path.to_string_lossy().into_owned())
    }).unwrap_or_else(|| "java".to_string());

    let entry_jar = match entry_jar_path(&manifest.libraries, libraries_dir(), versions_dir(), &mc_version, &mod_loader) {
        Some(p) => p,
        None => fail!(Error::Invalid(format!(
            "could not resolve entry jar for mod loader {mod_loader} \
             (missing loader entry in manifest libraries?)"
        ))),
    };
    if !entry_jar.exists() {
        fail!(Error::NotExists(entry_jar.to_string_lossy().into_owned()));
    }

    let asset_index = match manifest.asset_index.as_ref() {
        Some(idx) => idx,
        None => fail!(Error::Invalid("assetIndex not found in version JSON".into())),
    };
    log!(format!("Downloading assets ({})...", asset_index.id));
    if let Err(e) = download_assets(assets_dir(), asset_index, &state.http_client).await {
        fail!(e);
    }

    let natives_dir = versions_dir().join(&version_id).join("natives");
    log!("Extracting natives...");
    if let Err(e) = extract_natives(&manifest.libraries, libraries_dir(), &natives_dir) {
        log!(format!("Warning: natives: {e}"));
    }

    if manifest.main_class.is_empty() {
        fail!(Error::Invalid("mainClass not found in version JSON".into()));
    }

    let (classpath, missing_libs) = build_classpath(&manifest.libraries, libraries_dir(), versions_dir(), &mc_version, &mod_loader);
    for m in &missing_libs {
        log!(format!("Warning: manifest library missing on disk: {m}"));
    }
    if classpath.is_empty() {
        fail!(Error::Invalid("classpath is empty — libraries may not have downloaded correctly".into()));
    }

    let game_dir = PathBuf::from(&instance_location);
    std::fs::create_dir_all(&game_dir)?;
    let game_dir_str = game_dir.to_string_lossy().into_owned();
    let natives_str  = natives_dir.to_string_lossy().into_owned();
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
        // Deduplicate JVM args: parent + child manifests can each independently
        // set the same flag (e.g. `-XX:+UseCompactObjectHeaders` from vanilla's
        // default-user-jvm and Forge's jvm list). Keep the first occurrence.
        launch_args.extend(dedup_preserve_order(jvm_section));

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
