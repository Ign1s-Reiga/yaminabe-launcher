use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;
use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModLoader {
    Vanilla,
    Forge,
    Fabric,
    Quilt,
    NeoForge,
}

impl Display for ModLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModLoader::Vanilla => write!(f, "Vanilla"),
            ModLoader::Forge => write!(f, "Forge"),
            ModLoader::Fabric => write!(f, "Fabric"),
            ModLoader::Quilt => write!(f, "Quilt"),
            ModLoader::NeoForge => write!(f, "NeoForge"),
        }
    }
}

impl FromStr for ModLoader {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "vanilla" => Ok(ModLoader::Vanilla),
            "forge" => Ok(ModLoader::Forge),
            "fabric" => Ok(ModLoader::Fabric),
            "quilt" => Ok(ModLoader::Quilt),
            "neoforge" => Ok(ModLoader::NeoForge),
            other => Err(Error::Unsupported(format!("mod loader '{other}'"))),
        }
    }
}

impl ModLoader {
    pub fn get_modloader_color(&self) -> &'static str {
        match self {
            ModLoader::Vanilla => "#406b50",
            ModLoader::Forge => "#6b5040",
            ModLoader::Fabric => "#40506b",
            ModLoader::Quilt => "#50406b",
            ModLoader::NeoForge => "#6b5b40",
        }
    }
}

/// Where a downloadable artifact comes from — used for both an instance's
/// provenance (`InstanceMeta.origin`) and an individual mod file
/// (`ModListEntry.source`). Each remote variant carries the ids that fully
/// identify what to fetch, so one value answers "what, and from where".
/// `Manual` means there is no managed remote source: a user-assembled instance,
/// or a mod the user must install by hand (e.g. an automated download that
/// could not be resolved).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DownloadSource {
    #[default]
    Manual,
    CurseForge { project_id: u32, file_id: u32 },
    Modrinth { project_id: String, version_id: String },
}

impl DownloadSource {
    /// Whether the contents are managed by an external source and should
    /// therefore be treated as read-only in the UI. `false` only for `Manual`.
    pub fn is_managed(&self) -> bool {
        !matches!(self, DownloadSource::Manual)
    }

    /// The `(project_id, file_id)` of a CurseForge artifact, used to drive the
    /// modpack upgrade flow. `None` for any other source.
    pub fn curseforge_ids(&self) -> Option<(u32, u32)> {
        match self {
            DownloadSource::CurseForge { project_id, file_id } => Some((*project_id, *file_id)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceMeta {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub mod_loader: ModLoader,
    pub mod_loader_version: Option<String>,
    #[serde(default)]
    pub version_id: String,
    pub category: String,
    pub ram_mb: u32,
    pub jvm_args: String,
    #[serde(default)]
    pub jre_path: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub window_width: u32,
    #[serde(default)]
    pub window_height: u32,
    #[serde(default)]
    pub origin: DownloadSource,
}

impl Default for InstanceMeta {
    fn default() -> Self {
        // `SystemTime::now()` panics on `wasm32-unknown-unknown`; the backend
        // regenerates the id whenever it is empty.
        #[cfg(not(target_family = "wasm"))]
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .to_string();
        #[cfg(target_family = "wasm")]
        let id = String::new();

        Self {
            id,
            name: String::new(),
            game_version: String::new(),
            mod_loader: ModLoader::Vanilla,
            mod_loader_version: None,
            version_id: String::new(),
            category: String::new(),
            ram_mb: 4096,
            jvm_args: String::new(),
            jre_path: String::new(),
            description: String::new(),
            window_width: 0,
            window_height: 0,
            origin: DownloadSource::Manual,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModProjectInfo {
    pub id: u32,
    #[serde(default)]
    pub file_id: Option<u32>,
    pub name: String,
    pub summary: String,
    pub logo_url: Option<String>,
    pub download_count: u32,
    pub game_versions: Vec<String>,
    pub category: Vec<String>,
    pub primary_category_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModProjectSearchResults {
    pub items: Vec<ModProjectInfo>,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModProjectFile {
    pub id: u32,
    pub mod_id: u32,
    pub release_type: String,
    pub file_name: String,
    pub download_url: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModListEntry {
    pub file_name: String,
    pub sha1: String,
    /// Where this jar was fetched from. `#[serde(default)]` so a modlist.json
    /// written before the field existed deserializes as `Manual`.
    #[serde(default)]
    pub source: DownloadSource,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub curseforge_api_key: String,
    pub jvm_args: String,
    pub memory_mb: u32,
    #[serde(default)]
    pub instance_install_dir: String,
    #[serde(default)]
    pub window_width: u32,
    #[serde(default)]
    pub window_height: u32,
    /// Id of the most recently launched instance. Drives the navbar's
    /// Instant-Play button so the user can relaunch without opening the library.
    #[serde(default)]
    pub last_played_instance_id: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            curseforge_api_key: String::new(),
            jvm_args: String::new(),
            memory_mb: 4096,
            instance_install_dir: String::new(),
            window_width: 0,
            window_height: 0,
            last_played_instance_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaInstall {
    pub path: String,
    pub version: String,
    pub vendor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseType {
    Release,
    Snapshot,
    #[serde(rename = "old_beta")]
    Beta,
    #[serde(rename = "old_alpha")]
    Alpha,
}

impl Display for ReleaseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReleaseType::Release => write!(f, "Release"),
            ReleaseType::Snapshot => write!(f, "Snapshot"),
            ReleaseType::Beta => write!(f, "Beta"),
            ReleaseType::Alpha => write!(f, "Alpha"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameVersion {
    pub id: u32,
    #[serde(rename = "versionString")]
    pub version_string: String,
    #[serde(rename = "releaseType")]
    pub release_type: ReleaseType,
}

/// Minimal view of a saved Microsoft account that is safe to ship to the
/// frontend — no tokens, just the public profile identifiers the UI needs to
/// render the account list and select an active player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub uuid: String,
    pub username: String,
}

/// Launch authentication mode chosen from the split Play button. `Online`
/// uses the currently-selected Microsoft account (falling back to offline if
/// none); `Offline` ignores the selected account entirely.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LaunchMode {
    Online,
    Offline,
}

impl LaunchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            LaunchMode::Online => "online",
            LaunchMode::Offline => "offline",
        }
    }

    pub fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("offline") => LaunchMode::Offline,
            _ => LaunchMode::Online,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderVersion {
    pub id: u32,
    pub version: String,
    pub game_version: String,
    #[serde(rename = "type")]
    pub loader_type: String,
}

#[cfg(test)]
mod tests {
    use super::{DownloadSource, ModListEntry};

    #[test]
    fn serializes_manual_source_as_typed_object() {
        let json = serde_json::to_string(&DownloadSource::Manual).unwrap();

        assert_eq!(json, r#"{"type":"manual"}"#);
    }

    #[test]
    fn serializes_curseforge_source_as_typed_object() {
        let json = serde_json::to_string(&DownloadSource::CurseForge {
            project_id: 1198207,
            file_id: 8186745,
        }).unwrap();

        assert_eq!(json, r#"{"type":"curseForge","project_id":1198207,"file_id":8186745}"#);
    }

    #[test]
    fn deserializes_typed_object_sources() {
        let manual: DownloadSource = serde_json::from_str(
            r#"{"type":"manual"}"#,
        ).unwrap();
        let curseforge: DownloadSource = serde_json::from_str(
            r#"{"type":"curseForge","project_id":1198207,"file_id":8186745}"#,
        ).unwrap();
        let modrinth: DownloadSource = serde_json::from_str(
            r#"{"type":"modrinth","project_id":"AANobbMI","version_id":"IIJjncsf"}"#,
        ).unwrap();

        assert_eq!(manual, DownloadSource::Manual);
        assert_eq!(curseforge, DownloadSource::CurseForge {
            project_id: 1198207,
            file_id: 8186745,
        });
        assert_eq!(modrinth, DownloadSource::Modrinth {
            project_id: "AANobbMI".to_string(),
            version_id: "IIJjncsf".to_string(),
        });
    }

    #[test]
    fn serializes_modlist_entry_schema() {
        let json = serde_json::to_string(&ModListEntry {
            file_name: "example.jar".to_string(),
            sha1: "abc123".to_string(),
            source: DownloadSource::CurseForge { project_id: 10, file_id: 20 },
            size: 1024,
        }).unwrap();

        assert_eq!(
            json,
            r#"{"fileName":"example.jar","sha1":"abc123","source":{"type":"curseForge","project_id":10,"file_id":20},"size":1024}"#
        );
    }
}
