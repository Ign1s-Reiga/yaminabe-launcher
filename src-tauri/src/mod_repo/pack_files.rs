use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::commands::instance::{is_current_dir, is_launcher_dir, is_parent_dir, modlist_file};
use crate::http_utils::sha1_hex;
use crate::json::read_json_or_default;
use log::warn;
use yaminabe_launcher_shared::datamodels::{ModListEntry, ProjectFileTarget, ProjectId};

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

/// What the instance already had, by file name, so an upgrade can tell an
/// unchanged file from a new one.
pub fn installed_entries(instance_path: &Path) -> HashMap<String, ModListEntry> {
    let modlist: Vec<ModListEntry> =
        read_json_or_default(modlist_file(instance_path)).unwrap_or_default();
    modlist
        .into_iter()
        .map(|entry| (entry.file_name.clone(), entry))
        .collect()
}

/// How a path compares for "is this still shipped".
///
/// Windows and macOS both reach one file by either spelling by default, so two
/// cases are one file there and comparing them exactly would delete the file
/// just written. A case-sensitive filesystem is the opposite: treating them as
/// one leaves both spellings on disk, which the loader reads as a duplicate
/// mod. So the comparison follows the platform rather than picking a side.
///
/// The platform is a stand-in for the filesystem, which is what actually
/// decides; a case-sensitive volume on macOS, or a case-insensitive one on
/// Linux, is judged by its host's usual behaviour rather than its own.
pub fn path_key(relative: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        relative.to_lowercase()
    } else {
        relative.to_string()
    }
}

/// Where a disabled mod actually sits on disk.
pub fn disabled_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".disabled");
    dest.with_file_name(name)
}

/// Whether this kind of file is the launcher's to toggle with a `.disabled`
/// name.
///
/// Only mods are. Elsewhere that suffix is the user's own name for their own
/// file and none of the pack's business: `theme.zip.disabled` beside a resource
/// pack is a backup, and treating it as a toggle deletes it and moves the pack's
/// own file out of the way.
pub fn uses_disabled_suffix(target: Option<ProjectFileTarget>) -> bool {
    target == Some(ProjectFileTarget::Mod)
}

/// The `.disabled` name the launcher toggles the file at `relative` with: only
/// a mod has one, and only while the pack has not claimed the name itself.
///
/// A pack may ship `foo.jar` and `foo.jar.disabled` side by side, as one does
/// to offer an alternate build. That second file is the pack's own, and taking
/// it for the first one's twin — renaming over it, clearing it — destroys it.
///
/// `claimed` is what one version of the pack installs, keyed by [`path_key`].
pub fn toggle_twin(
    dest: &Path,
    relative: &str,
    target: Option<ProjectFileTarget>,
    claimed: &HashSet<String>,
) -> Option<PathBuf> {
    let taken = claimed.contains(&path_key(&format!("{relative}.disabled")));
    (uses_disabled_suffix(target) && !taken).then(|| disabled_path(dest))
}

/// The mod-list row that speaks for the file at `relative`, if one does.
///
/// The list keys on bare names, so a row can only stand for a file sitting
/// directly in its own target's directory. Anything else sharing that name —
/// `config/common.jar` beside `mods/common.jar`, or a nested `mods/sub/a.jar` —
/// is a different file, and must not inherit its state or its hash.
pub fn recorded_entry<'a>(
    previous: &'a HashMap<String, ModListEntry>,
    relative: &str,
) -> Option<&'a ModListEntry> {
    let name = relative.rsplit('/').next()?;
    previous
        .get(name)
        .filter(|entry| relative == format!("{}/{}", entry.target.directory(), name))
}

/// Whether the mod at `dest` is off as the instance stands: its twin is there
/// and the plain jar is not, since the loader reads the plain one.
///
/// A twin the old version shipped as a file of its own (`pack_before`) is the
/// user's choice only once its bytes are shown to be this mod's, against the
/// hash the mod list recorded: the toggle renames the mod over whatever sat
/// there. A mod with no recorded hash, such as one that only arrived as an
/// override, cannot be told from the pack's file and is read as on.
pub fn is_turned_off(
    dest: &Path,
    relative: &str,
    entry: Option<&ModListEntry>,
    pack_before: &HashSet<String>,
) -> bool {
    let twin = disabled_path(dest);
    if dest.exists() || !twin.exists() {
        return false;
    }
    if toggle_twin(dest, relative, target_for(relative), pack_before).is_some() {
        return true;
    }
    entry.filter(|entry| !entry.sha1.is_empty()).is_some_and(|entry| {
        std::fs::read(&twin).is_ok_and(|bytes| sha1_hex(&bytes).eq_ignore_ascii_case(&entry.sha1))
    })
}

/// Which mods the user had turned off, read before an upgrade fetches or
/// removes anything — both rewrite the files it reads — so that each mod's new
/// version can be left the same way.
///
/// Kept by where each file sat and by the project the mod list recorded for it:
/// a new version usually arrives under a new file name, and only the project
/// says it is the same mod.
#[derive(Default)]
pub struct ModStates {
    /// Each mod's path key, and whether it was off.
    paths: HashMap<String, bool>,
    /// Each recorded project, and whether every file of it was off.
    projects: HashMap<ProjectId, bool>,
}

impl ModStates {
    /// Read every mod the record or the mod list knows of, including ones the
    /// user added by hand. `installed_before` is what the old version of the
    /// pack put there, where that was recorded.
    pub fn read(
        instance_path: &Path,
        previous: &HashMap<String, ModListEntry>,
        installed_before: &[String],
    ) -> Self {
        let pack_before: HashSet<String> = installed_before.iter().map(|path| path_key(path)).collect();
        let listed = previous
            .values()
            .map(|entry| format!("{}/{}", entry.target.directory(), entry.file_name));
        let mut states = Self::default();
        for relative in listed.chain(installed_before.iter().cloned()) {
            // A `.disabled` path a pack installs is its own file, shipped off;
            // it has no state of its own to carry.
            if relative.ends_with(".disabled") || !uses_disabled_suffix(target_for(&relative)) {
                continue;
            }
            let Some(dest) = safe_destination(instance_path, &relative) else { continue };
            let entry = recorded_entry(previous, &relative);
            let off = is_turned_off(&dest, &relative, entry, &pack_before);
            if states.paths.insert(path_key(&relative), off).is_some() {
                continue;
            }
            // Mixed states leave no choice to carry: only a project the user
            // turned off entirely is turned off again.
            if let Some(project) = entry.and_then(|entry| entry.source.project_id()) {
                *states.projects.entry(project).or_insert(true) &= off;
            }
        }
        states
    }

    /// Whether the user had the mod at `relative` off: the file at that same
    /// path if there was one, otherwise the old version of `project`.
    pub fn was_disabled(&self, relative: &str, project: Option<&ProjectId>) -> bool {
        self.paths
            .get(&path_key(relative))
            .or_else(|| project.and_then(|id| self.projects.get(id)))
            .copied()
            .unwrap_or(false)
    }
}

/// Give each mod the new version installed the state the user left its old
/// version in, and return the path keys of those now off.
///
/// Run last, once the old version is gone and the overrides are written, so
/// nothing after it can remove or overwrite what it renames. `mods` pairs each
/// path with the project its file belongs to, where that is known. A mod whose
/// twin name the new version ships as a file of its own is left on.
pub fn align_states<'a>(
    instance_path: &Path,
    mods: impl IntoIterator<Item = (&'a str, Option<ProjectId>)>,
    states: &ModStates,
    claimed: &HashSet<String>,
) -> HashSet<String> {
    let mut turned_off = HashSet::new();
    for (relative, project) in mods {
        if !states.was_disabled(relative, project.as_ref()) {
            continue;
        }
        let Some(dest) = safe_destination(instance_path, relative) else { continue };
        let Some(twin) = toggle_twin(&dest, relative, target_for(relative), claimed) else {
            warn!("cannot turn {relative} off: the pack ships its disabled name itself");
            continue;
        };
        if dest.exists() {
            // Anything at the twin is the old version this one replaces.
            std::fs::remove_file(&twin).ok();
            if let Err(e) = std::fs::rename(&dest, &twin) {
                warn!("cannot turn {relative} off: {e}; leaving it on");
                continue;
            }
        } else if !twin.exists() {
            continue;
        }
        turned_off.insert(path_key(relative));
    }
    turned_off
}

#[cfg(test)]
mod pack_files_tests {
    use super::{
        align_states, disabled_path, is_turned_off, path_key, safe_destination, target_for, ModStates,
    };
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use yaminabe_launcher_shared::datamodels::{
        DownloadSource, ModListEntry, ModState, ProjectFileTarget, ProjectId,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yaminabe-pack-files-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("mods")).expect("create mods dir");
        dir
    }

    fn entry(file_name: &str, sha1: &str, state: ModState) -> ModListEntry {
        ModListEntry {
            file_name: file_name.to_string(),
            project_name: String::new(),
            icon_url: None,
            sha1: sha1.to_string(),
            source: DownloadSource::Manual,
            target: ProjectFileTarget::Mod,
            size: 0,
            state,
        }
    }

    /// `entry`, as the mod list records it once Modrinth has named its project.
    fn of_project(entry: ModListEntry, project_id: &str) -> ModListEntry {
        ModListEntry {
            source: DownloadSource::Modrinth {
                project_id: project_id.to_string(),
                version_id: String::new(),
            },
            ..entry
        }
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

    #[test]
    fn a_disabled_mod_is_named_for_where_it_actually_sits() {
        assert_eq!(
            disabled_path(Path::new("/i/mods/a.jar")),
            PathBuf::from("/i/mods/a.jar.disabled")
        );
    }

    /// Windows reaches one file by either spelling and Linux does not, so the
    /// comparison has to follow the platform: one way deletes the file just
    /// written, the other leaves both spellings for the loader to trip over.
    #[test]
    fn path_comparison_follows_the_filesystem() {
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(path_key("mods/Foo.jar"), path_key("mods/foo.jar"));
        } else {
            assert_ne!(path_key("mods/Foo.jar"), path_key("mods/foo.jar"));
        }
    }

    /// What has to hold for a mod to read as off. Each has been got wrong on its
    /// own, in a different place that was asking the question for itself.
    #[test]
    fn a_mod_reads_as_off_only_by_its_own_twin() {
        let dir = temp_dir("twin-ownership");
        std::fs::create_dir_all(dir.join("resourcepacks")).expect("create dir");
        let mod_dest = dir.join("mods").join("a.jar");
        let pack_dest = dir.join("resourcepacks").join("theme.zip");
        std::fs::write(disabled_path(&mod_dest), b"turned off").expect("write");
        std::fs::write(disabled_path(&pack_dest), b"the user's backup").expect("write");
        let none = HashSet::new();

        assert!(is_turned_off(&mod_dest, "mods/a.jar", None, &none), "a mod's twin is the user's toggle");
        // Elsewhere the suffix is the user's own name for their own file.
        assert!(
            !is_turned_off(&pack_dest, "resourcepacks/theme.zip", None, &none),
            "a backup beside a resource pack is not a toggle"
        );
        // A twin the old version shipped is its own file, unless its bytes show
        // the user's toggle renamed this mod over it.
        let pack_before = HashSet::from([path_key("mods/a.jar.disabled")]);
        let other = entry("a.jar", "0ld0ld", ModState::Enabled);
        let same = entry("a.jar", &crate::http_utils::sha1_hex(b"turned off"), ModState::Disabled);
        assert!(
            !is_turned_off(&mod_dest, "mods/a.jar", Some(&other), &pack_before),
            "the pack's alternate build"
        );
        assert!(
            is_turned_off(&mod_dest, "mods/a.jar", Some(&same), &pack_before),
            "the mod itself, turned off over it"
        );
        // The loader reads the plain jar, so with both there the mod is on.
        std::fs::write(&mod_dest, b"on").expect("write");
        assert!(!is_turned_off(&mod_dest, "mods/a.jar", None, &none), "the plain jar wins");
        // And with no twin there is no choice to read.
        std::fs::remove_file(disabled_path(&mod_dest)).expect("remove");
        std::fs::remove_file(&mod_dest).expect("remove");
        assert!(!is_turned_off(&mod_dest, "mods/a.jar", None, &none));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The old version shipped `a.jar.disabled` as an alternate build, and
    /// `a.jar` itself never arrived — the one way that file sits there alone
    /// without the user's toggle. Read as a choice, the new version of `a.jar`
    /// would be turned off.
    #[test]
    fn a_twin_the_old_version_shipped_is_not_the_user_s_choice() {
        let dir = temp_dir("old-version-twin");
        std::fs::write(dir.join("mods").join("a.jar.disabled"), b"the old alternate build").expect("write twin");
        let previous = HashMap::from([(
            "a.jar".to_string(),
            of_project(entry("a.jar", "aaaa", ModState::DownloadFailed), "P"),
        )]);
        let installed_before = vec!["mods/a.jar".to_string(), "mods/a.jar.disabled".to_string()];

        let states = ModStates::read(&dir, &previous, &installed_before);

        assert!(!states.was_disabled("mods/a.jar", None));
        assert!(!states.was_disabled("mods/a-2.0.jar", Some(&ProjectId::Modrinth("P".to_string()))), "nor is its project");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Without a project to go by — a hand-added mod, or a lookup that failed —
    /// a mod is matched by its path. The old disabled copy there is the version
    /// being replaced, and the new one takes its place under the same name.
    #[test]
    fn a_mod_without_a_project_is_matched_by_its_path() {
        let dir = temp_dir("by-path");
        let jar = dir.join("mods").join("a.jar");
        std::fs::write(disabled_path(&jar), b"old").expect("write old");
        let previous = HashMap::from([("a.jar".to_string(), entry("a.jar", "0ld0ld", ModState::Disabled))]);
        let states = ModStates::read(&dir, &previous, &["mods/a.jar".to_string()]);
        std::fs::write(&jar, b"new").expect("write new");
        let claimed = HashSet::from([path_key("mods/a.jar")]);

        let off = align_states(&dir, [("mods/a.jar", None)], &states, &claimed);

        assert!(!jar.exists());
        assert_eq!(std::fs::read(disabled_path(&jar)).expect("read"), b"new", "the new version, turned off");
        assert!(off.contains(&path_key("mods/a.jar")));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two files of one project, one turned off and one not, leave no single
    /// choice to carry, so a new file of that project is left on.
    #[test]
    fn a_project_is_turned_off_only_when_all_its_files_were() {
        let dir = temp_dir("mixed-project");
        std::fs::write(dir.join("mods").join("a.jar.disabled"), b"a").expect("write a");
        std::fs::write(dir.join("mods").join("b.jar"), b"b").expect("write b");
        let previous = HashMap::from([
            ("a.jar".to_string(), of_project(entry("a.jar", "aaaa", ModState::Disabled), "P")),
            ("b.jar".to_string(), of_project(entry("b.jar", "bbbb", ModState::Enabled), "P")),
        ]);

        let states = ModStates::read(&dir, &previous, &[]);

        assert!(states.was_disabled("mods/a.jar", None), "each path keeps its own state");
        assert!(!states.was_disabled("mods/c.jar", Some(&ProjectId::Modrinth("P".to_string()))));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// When the new version ships `a.jar.disabled` as a file of its own, the
    /// name is taken: turning `a.jar` off would destroy the pack's file, so the
    /// user's choice gives way and the mod is left on.
    #[test]
    fn a_mod_is_not_turned_off_onto_a_name_the_new_version_ships() {
        let dir = temp_dir("taken-twin");
        let jar = dir.join("mods").join("a.jar");
        std::fs::write(disabled_path(&jar), b"old").expect("write old");
        let previous = HashMap::from([("a.jar".to_string(), entry("a.jar", "0ld0ld", ModState::Disabled))]);
        let states = ModStates::read(&dir, &previous, &["mods/a.jar".to_string()]);
        // The new version's files: the mod, and its alternate build.
        std::fs::write(&jar, b"new").expect("write new");
        std::fs::write(disabled_path(&jar), b"the pack's alternate build").expect("write alternate");
        let claimed = HashSet::from([path_key("mods/a.jar"), path_key("mods/a.jar.disabled")]);

        let off = align_states(&dir, [("mods/a.jar", None)], &states, &claimed);

        assert!(jar.exists(), "left on");
        assert_eq!(std::fs::read(disabled_path(&jar)).expect("read"), b"the pack's alternate build");
        assert!(off.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
