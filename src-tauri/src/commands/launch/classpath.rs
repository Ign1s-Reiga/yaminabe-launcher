use std::collections::HashSet;
use std::path::{Path, PathBuf};
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::datamodels::Library;
use crate::install_task::maven_coord_to_path;
use super::args::eval_rules;

pub(super) fn extract_natives(
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
        // Native dir is just the group/artifact/version segment of the maven
        // coord — same layout as `maven_coord_to_path` without the filename.
        let Some(native_dir_rel) = maven_coord_to_path(&lib.name).parent().map(Path::to_path_buf) else { continue; };
        let Some(classifiers) = lib.downloads.as_ref().and_then(|d| d.classifiers.as_ref()) else { continue; };
        let Some(artifact) = classifiers.get(&key) else { continue; };
        let Some(path) = &artifact.path else { continue; };
        let jar_path = libraries_dir.join(path);
        if !jar_path.exists() { continue; }
        let Ok(file)    = std::fs::File::open(&jar_path) else { continue; };
        let Ok(mut zip) = zip::ZipArchive::new(file) else { continue; };
        let native_dir = natives_root.join(&native_dir_rel);
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

/// Resolve a library's on-disk path. Prefers the manifest's explicit
/// `downloads.artifact.path` when present, falling back to the derived
/// maven-coordinate path. The returned path may not exist on disk — callers
/// check `exists()` before using it.
fn library_path(lib: &Library, libraries_dir: &Path) -> PathBuf {
    if let Some(path) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()).and_then(|a| a.path.as_deref()) {
        libraries_dir.join(path)
    } else {
        libraries_dir.join(maven_coord_to_path(&lib.name))
    }
}

pub(super) fn jar_contains_class(jar_path: &Path, main_class: &str) -> bool {
    let class_path = format!("{}.class", main_class.replace('.', "/"));
    let Ok(file) = std::fs::File::open(jar_path) else { return false; };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return false; };
    zip.by_name(&class_path).is_ok()
}

pub(super) fn find_main_class_jar(
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
        let path = library_path(lib, libraries_dir);
        if path.exists() && jar_contains_class(&path, main_class) {
            return Some(path);
        }
    }
    None
}

/// Build the dedup key for a library at the `group:artifact[:classifier]`
/// granularity, ignoring the version. Merged manifests routinely carry
/// multiple versions of the same artifact (e.g. when a loader overrides a
/// vanilla library); collapsing on this key keeps the first occurrence (in
/// our merge order, the child's preference).
fn version_agnostic_name(maven_coord: &str) -> String {
    let parts: Vec<&str> = maven_coord.splitn(4, ':').collect();
    let group = parts.first().copied().unwrap_or("");
    let artifact = parts.get(1).copied().unwrap_or("");
    match parts.get(3) {
        Some(classifier) => format!("{group}:{artifact}:{classifier}"),
        None => format!("{group}:{artifact}"),
    }
}

pub(super) fn build_classpath(
    libraries: &[Library],
    libraries_dir: &Path,
    versions_dir: &Path,
    mc_version: &str,
    version_id: &str,
) -> (String, Vec<String>) {
    // Skip natives libs (extract_natives handles those) and dedupe by
    // `group:artifact[:classifier]` so the loader's preferred version (first
    // in the merge order) wins. `missing` surfaces absent libs to the caller.
    let mut seen: HashSet<String> = HashSet::new();
    let mut paths: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    // Only Vanilla instances have a per-version primary jar; Forge/NeoForge
    // ship theirs under `libraries/` as an ordinary entry.
    let primary_jar = versions_dir.join(version_id).join(format!("{version_id}.jar"));
    if primary_jar.exists() {
        paths.push(primary_jar.to_string_lossy().into_owned());
    }
    for lib in libraries {
        if !eval_rules(&lib.rules) { continue; }
        if !lib.natives.is_empty() { continue; }
        let key = version_agnostic_name(&lib.name);
        if !seen.insert(key) { continue; }
        let p = library_path(lib, libraries_dir);
        if p.exists() {
            paths.push(p.to_string_lossy().into_owned());
        } else {
            missing.push(format!("{} (expected at {})", lib.name, p.display()));
        }
    }
    // Non-Vanilla loaders (Fabric, Quilt, pre-1.13 Forge) need the vanilla
    // client jar on classpath since their patched jars don't bundle it.
    if version_id != mc_version {
        let vanilla_jar = versions_dir.join(mc_version).join(format!("{mc_version}.jar"));
        if vanilla_jar.exists() { paths.push(vanilla_jar.to_string_lossy().into_owned()); }
    }
    (paths.join(";"), missing)
}