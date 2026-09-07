use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::commands::instance::{is_current_dir, is_launcher_dir, is_parent_dir};
use serde::Deserialize;
use yaminabe_launcher_shared::datamodels::{
    LocalModpackInfo, ModLoader, ModpackFormat, ProjectFileTarget,
};
use yaminabe_launcher_shared::error::Error;

/// The index at the root of a `.mrpack`, Modrinth's modpack format.
///
/// Unlike a CurseForge manifest, this carries each file's download URL and hash
/// outright, so installing from it needs no API and no key.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackIndex {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub files: Vec<MrpackFile>,
    /// Maps `minecraft` and one loader to their versions.
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    /// Where the file belongs, relative to the instance root.
    pub path: String,
    #[serde(default)]
    pub hashes: MrpackHashes,
    #[serde(default)]
    pub env: Option<MrpackEnv>,
    /// Mirrors to try in order; the first that works wins.
    #[serde(default)]
    pub downloads: Vec<String>,
    /// What the index says the file weighs. Recorded even for a file that could
    /// not be fetched, so the Mods tab can show what is missing rather than 0 B.
    #[serde(default)]
    pub file_size: u64,
}

#[derive(Debug, Default, Deserialize)]
pub struct MrpackHashes {
    #[serde(default)]
    pub sha1: String,
}

#[derive(Debug, Deserialize)]
pub struct MrpackEnv {
    /// `required`, `optional` or `unsupported` — a launcher installs a client.
    #[serde(default)]
    pub client: String,
}

impl MrpackFile {
    /// Whether a client install wants this file. Only an explicit `unsupported`
    /// excludes it; an optional file is still installed, since a pack ships the
    /// set it expects to run with.
    pub fn wanted_by_client(&self) -> bool {
        !matches!(self.env.as_ref().map(|env| env.client.as_str()), Some("unsupported"))
    }
}

pub const INDEX_NAME: &str = "modrinth.index.json";


pub fn read_index<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<MrpackIndex, Error> {
    let entry = archive
        .by_name(INDEX_NAME)
        .map_err(|_| Error::Invalid(format!("modpack zip is missing {INDEX_NAME}")))?;
    serde_json::from_reader(entry).map_err(Error::from)
}

/// The loader a pack asks for. Modrinth keys the loader by its own name, and
/// spells the two that end in `-loader` differently from how the launcher does.
pub fn resolve_loader(dependencies: &HashMap<String, String>) -> (ModLoader, Option<String>) {
    for (key, loader) in [
        ("neoforge", ModLoader::NeoForge),
        ("forge", ModLoader::Forge),
        ("fabric-loader", ModLoader::Fabric),
        ("quilt-loader", ModLoader::Quilt),
    ] {
        if let Some(version) = dependencies.get(key) {
            return (loader, Some(version.clone()));
        }
    }
    (ModLoader::Vanilla, None)
}

/// Resolve a path from the index against `instance_path`, refusing one that
/// would land outside it. The index is attacker-controlled in the same way a
/// zip's entry names are: `..` climbs out, and a drive prefix discards the base
/// entirely on Windows.
pub fn safe_destination(instance_path: &Path, relative: &str) -> Option<PathBuf> {
    let mut components: Vec<&str> = Vec::new();
    for part in relative.split(['/', '\\']) {
        // Refused, not filtered out. Dropping a `..` silently turns
        // `mods/../evil.jar` into `mods/evil.jar` and writes it — a path that
        // tries to leave is not one to rewrite into a path that stays.
        if is_parent_dir(part) || part.contains(':') {
            return None;
        }
        if !is_current_dir(part) {
            components.push(part);
        }
    }
    if components.is_empty() {
        return None;
    }
    // `.launcher/` is the launcher's own record of the instance, not part of
    // the pack. Nothing needs `..` to reach it, and a modlist planted there
    // survives the install and renders whatever it likes in the Mods tab.
    if is_launcher_dir(components[0]) {
        return None;
    }
    Some(
        components
            .iter()
            .fold(instance_path.to_path_buf(), |path, part| path.join(part)),
    )
}

/// Which tracked kind a path belongs to, so a failed download can be linked by
/// hand into the right place. `None` for a path the modlist does not model.
pub fn target_for(relative: &str) -> Option<ProjectFileTarget> {
    // Normalised the same way safe_destination does, so `./mods/x.jar` is not
    // read as a different directory from `mods/x.jar` and left untracked.
    let first = relative
        .split(['/', '\\'])
        .find(|part| !is_current_dir(part) && !is_parent_dir(part) && !part.contains(':'))?;
    match first {
        "mods" => Some(ProjectFileTarget::Mod),
        "resourcepacks" => Some(ProjectFileTarget::ResourcePack),
        "shaderpacks" => Some(ProjectFileTarget::ShaderPack),
        "datapacks" => Some(ProjectFileTarget::DataPack),
        _ => None,
    }
}

/// Describe a `.mrpack` without installing it.
pub fn read_local_modpack(zip_path: &Path) -> Result<LocalModpackInfo, Error> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| Error::Invalid(format!("cannot open {}: {e}", zip_path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| Error::Invalid(format!("{} is not a zip file", zip_path.display())))?;
    let index = read_index(&mut archive)?;
    let (mod_loader, mod_loader_version) = resolve_loader(&index.dependencies);
    Ok(LocalModpackInfo {
        format: ModpackFormat::Modrinth,
        name: index.name,
        version: index.version_id,
        // The format carries no author field; a summary is the closest thing to
        // show, and an empty string simply renders nothing.
        author: index.summary,
        game_version: index.dependencies.get("minecraft").cloned().unwrap_or_default(),
        mod_loader,
        mod_loader_version,
        file_count: index.files.len(),
    })
}

#[cfg(test)]
mod mrpack_tests {
    use super::{resolve_loader, safe_destination, target_for};
    use std::collections::HashMap;
    use std::path::Path;
    use yaminabe_launcher_shared::datamodels::{ModLoader, ProjectFileTarget};

    fn deps(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn reads_the_loader_modrinth_names() {
        let (loader, version) = resolve_loader(&deps(&[
            ("minecraft", "1.21.1"),
            ("fabric-loader", "0.16.9"),
        ]));

        assert_eq!(loader, ModLoader::Fabric);
        assert_eq!(version.as_deref(), Some("0.16.9"));
    }

    #[test]
    fn a_pack_with_no_loader_is_vanilla() {
        let (loader, version) = resolve_loader(&deps(&[("minecraft", "1.21.1")]));

        assert_eq!(loader, ModLoader::Vanilla);
        assert_eq!(version, None);
    }

    #[test]
    fn refuses_a_path_that_climbs_out_of_the_instance() {
        let root = Path::new("/instances/pack");

        // Refused outright rather than filtered down to a path that stays: a
        // `..` dropped silently turns `mods/../evil.jar` into a write the pack
        // never described.
        assert_eq!(safe_destination(root, "mods/../../evil.jar"), None);
        assert_eq!(safe_destination(root, "../evil.jar"), None);
        // A drive prefix would otherwise replace the base outright on Windows.
        assert_eq!(safe_destination(root, "C:/evil.jar"), None);
        assert_eq!(safe_destination(root, ".."), None);
        // An ordinary path still resolves, including a nested one.
        assert_eq!(safe_destination(root, "mods/sub/a.jar"), Some(root.join("mods").join("sub").join("a.jar")));
    }

    /// The launcher's own directory is refused however the pack spells it.
    /// Windows and macOS reach one directory by either case, and Windows drops
    /// trailing dots and spaces — so an exact match refuses `.launcher` and
    /// admits three spellings that land in the very same place.
    #[test]
    fn refuses_every_spelling_of_the_launcher_directory() {
        let root = Path::new("/instances/pack");

        for spelling in [".launcher", ".Launcher", ".LAUNCHER", ".launcher.", ".launcher "] {
            assert_eq!(
                safe_destination(root, &format!("{spelling}/instance.json")),
                None,
                "{spelling} reaches the launcher's own directory"
            );
        }
        // A directory that merely starts the same is the pack's to write.
        assert!(safe_destination(root, ".launcherpack/a.json").is_some());
    }

    /// The same trailing dots and spaces Windows drops before reaching
    /// `.launcher` also turn `.. ` into a climb, so the two guards have to read
    /// a component the same way. An exact `..` comparison refuses `../evil.jar`
    /// and admits `.. /evil.jar`, which lands in the very same place.
    #[test]
    fn refuses_every_spelling_of_a_climb() {
        let root = Path::new("/instances/pack");

        for spelling in ["..", ".. ", "...", ".. ."] {
            assert_eq!(
                safe_destination(root, &format!("mods/{spelling}/evil.jar")),
                None,
                "{spelling} climbs out of the instance"
            );
        }
        // A single dot names the directory it sits in, and descends nowhere.
        assert_eq!(
            safe_destination(root, "./mods/./a.jar"),
            Some(root.join("mods").join("a.jar"))
        );
        // Dots inside a name are part of it, not a climb.
        assert!(safe_destination(root, "mods/a..b.jar").is_some());
    }

    #[test]
    fn maps_a_path_to_what_the_modlist_models() {
        assert_eq!(target_for("mods/a.jar"), Some(ProjectFileTarget::Mod));
        assert_eq!(target_for("resourcepacks/b.zip"), Some(ProjectFileTarget::ResourcePack));
        assert_eq!(target_for("shaderpacks/c.zip"), Some(ProjectFileTarget::ShaderPack));
        assert_eq!(target_for("config/d.json"), None);
    }
}
