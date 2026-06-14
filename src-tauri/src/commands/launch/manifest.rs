use std::collections::HashSet;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::datamodels::ClientManifest;
use crate::install_task::version_manifest_path;

pub(super) fn load_manifest(version_id: &str) -> Result<ClientManifest, Error> {
    let text = std::fs::read_to_string(version_manifest_path(version_id))?;
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
pub(super) fn merge_manifest(version_id: &str) -> Result<ClientManifest, Error> {
    let mut manifest = load_manifest(version_id)?;
    let Some(parent_id) = manifest.inherits_from.take() else {
        return Ok(manifest);
    };
    let parent = load_manifest(&parent_id)?;

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