use super::minecraft::ModLoader;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Display;

/// Where a downloadable artifact comes from - used for both an instance's
/// provenance (`InstanceMeta.origin`) and an individual mod file
/// (`ModListEntry.source`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DownloadSource {
    #[default]
    Manual,
    CurseForge {
        project_id: u32,
        file_id: u32,
    },
    Modrinth {
        project_id: String,
        version_id: String,
    },
}

impl DownloadSource {
    pub fn is_managed(&self) -> bool {
        !matches!(self, DownloadSource::Manual)
    }

    pub fn curseforge_ids(&self) -> Option<(u32, u32)> {
        match self {
            DownloadSource::CurseForge {
                project_id,
                file_id,
            } => Some((*project_id, *file_id)),
            _ => None,
        }
    }
}

/// Which kind of project file is being searched, listed, or downloaded.
/// Directory values are instance-relative and are joined onto `instance_path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectFileTarget {
    #[default]
    Mod,
    ResourcePack,
    Modpack,
}

impl ProjectFileTarget {
    pub fn directory(&self) -> &'static str {
        match self {
            ProjectFileTarget::Mod => "mods",
            ProjectFileTarget::ResourcePack => "resourcepacks",
            ProjectFileTarget::Modpack => ".launcher/cache",
        }
    }

    pub fn tracks_modlist(&self) -> bool {
        matches!(self, ProjectFileTarget::Mod)
    }

    pub fn to_curseforge_type(&self) -> &'static str {
        match self {
            ProjectFileTarget::Mod => "6",
            ProjectFileTarget::ResourcePack => "12",
            ProjectFileTarget::Modpack => "4471",
        }
    }

    pub fn to_modrinth_type(&self) -> &'static str {
        match self {
            ProjectFileTarget::Mod => "mod",
            ProjectFileTarget::ResourcePack => "resourcepack",
            ProjectFileTarget::Modpack => "modpack",
        }
    }

    pub fn from_curseforge_type(project_type: u32) -> ProjectFileTarget {
        match project_type {
            12 => ProjectFileTarget::ResourcePack,
            4471 => ProjectFileTarget::Modpack,
            _ => ProjectFileTarget::Mod,
        }
    }

    pub fn from_modrinth_type(project_type: &str) -> ProjectFileTarget {
        match project_type {
            "resourcepack" => ProjectFileTarget::ResourcePack,
            "modpack" => ProjectFileTarget::Modpack,
            _ => ProjectFileTarget::Mod,
        }
    }
}

/// The mod-distribution platform a search targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    #[default]
    CurseForge,
    Modrinth,
}

/// Sort order for a project search. The CurseForge options the launcher
/// exposes, mapped per-platform (`sortField` for CurseForge, `index` for
/// Modrinth).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectSortField {
    #[default]
    Relevancy,
    Popularity,
    LatestUpdate,
    CreationDate,
    TotalDownloads,
}

impl ProjectSortField {
    /// CurseForge `sortField` value, or `""` for Relevancy (omit the param so
    /// the API ranks by search relevance).
    pub fn to_curseforge_field(&self) -> &'static str {
        match self {
            ProjectSortField::Relevancy => "",
            ProjectSortField::Popularity => "2",
            ProjectSortField::LatestUpdate => "3",
            ProjectSortField::CreationDate => "11",
            ProjectSortField::TotalDownloads => "6",
        }
    }

    /// Modrinth search `index` value.
    pub fn to_modrinth_index(&self) -> &'static str {
        match self {
            ProjectSortField::Relevancy => "relevance",
            ProjectSortField::Popularity => "follows",
            ProjectSortField::LatestUpdate => "updated",
            ProjectSortField::CreationDate => "newest",
            ProjectSortField::TotalDownloads => "downloads",
        }
    }

    /// Every variant, in display order — for building a sort selector.
    pub fn all() -> [ProjectSortField; 5] {
        [
            ProjectSortField::Relevancy,
            ProjectSortField::Popularity,
            ProjectSortField::LatestUpdate,
            ProjectSortField::CreationDate,
            ProjectSortField::TotalDownloads,
        ]
    }
}

impl Display for ProjectSortField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            ProjectSortField::Relevancy => "Relevancy",
            ProjectSortField::Popularity => "Popularity",
            ProjectSortField::LatestUpdate => "Latest Update",
            ProjectSortField::CreationDate => "Creation Date",
            ProjectSortField::TotalDownloads => "Total Downloads",
        };
        write!(f, "{label}")
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    pub query: String,
    pub index: u32,
    pub target: ProjectFileTarget,
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub mod_loader: Option<ModLoader>,
    #[serde(default)]
    pub sort: ProjectSortField,
    #[serde(default)]
    pub platform: Platform,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModProjectInfo {
    pub id: u32,
    #[serde(default)]
    pub platform: Platform,
    #[serde(default)]
    pub file_id: Option<u32>,
    pub name: String,
    pub summary: String,
    pub logo_url: Option<String>,
    pub download_count: u64,
    pub game_versions: Vec<String>,
    pub category: Vec<String>,
    pub primary_category_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModProjectSearchResults {
    pub items: Vec<ModProjectInfo>,
    pub total: u32,
}

/// A resolved project file: enough to display in a version picker and to
/// download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectFileInfo {
    pub source: DownloadSource,
    pub target: ProjectFileTarget,
    pub release_type: ProjectFileReleaseType,
    pub file_name: String,
    pub download_url: Option<String>,
    pub display_name: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
}

impl ProjectFileInfo {
    pub fn to_modlist_entry(&self, state: ModState) -> ModListEntry {
        ModListEntry {
            file_name: self.file_name.clone(),
            sha1: self.sha1.clone(),
            source: self.source.clone(),
            target: self.target,
            size: self.size,
            state,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectFileReleaseType {
    Release,
    Beta,
    Alpha,
    Unknown,
}

impl Display for ProjectFileReleaseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectFileReleaseType::Release => write!(f, "Release"),
            ProjectFileReleaseType::Beta => write!(f, "Beta"),
            ProjectFileReleaseType::Alpha => write!(f, "Alpha"),
            ProjectFileReleaseType::Unknown => write!(f, "Unknown"),
        }
    }
}

impl From<u32> for ProjectFileReleaseType {
    fn from(value: u32) -> Self {
        match value {
            1 => ProjectFileReleaseType::Release,
            2 => ProjectFileReleaseType::Beta,
            3 => ProjectFileReleaseType::Alpha,
            _ => ProjectFileReleaseType::Unknown,
        }
    }
}

impl From<String> for ProjectFileReleaseType {
    fn from(value: String) -> Self {
        match value.as_str() {
            "release" => ProjectFileReleaseType::Release,
            "beta" => ProjectFileReleaseType::Beta,
            "alpha" => ProjectFileReleaseType::Alpha,
            _ => ProjectFileReleaseType::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModState {
    #[default]
    Enabled,
    Disabled,
    DownloadFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModListEntry {
    pub file_name: String,
    pub sha1: String,
    #[serde(default)]
    pub source: DownloadSource,
    #[serde(default)]
    pub target: ProjectFileTarget,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub state: ModState,
}

#[cfg(test)]
mod tests {
    use super::{DownloadSource, ModListEntry, ModState};

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
        })
        .unwrap();

        assert_eq!(
            json,
            r#"{"type":"curseForge","project_id":1198207,"file_id":8186745}"#
        );
    }

    #[test]
    fn deserializes_typed_object_sources() {
        let manual: DownloadSource = serde_json::from_str(r#"{"type":"manual"}"#).unwrap();
        let curseforge: DownloadSource =
            serde_json::from_str(r#"{"type":"curseForge","project_id":1198207,"file_id":8186745}"#)
                .unwrap();
        let modrinth: DownloadSource = serde_json::from_str(
            r#"{"type":"modrinth","project_id":"AANobbMI","version_id":"IIJjncsf"}"#,
        )
        .unwrap();

        assert_eq!(manual, DownloadSource::Manual);
        assert_eq!(
            curseforge,
            DownloadSource::CurseForge {
                project_id: 1198207,
                file_id: 8186745,
            }
        );
        assert_eq!(
            modrinth,
            DownloadSource::Modrinth {
                project_id: "AANobbMI".to_string(),
                version_id: "IIJjncsf".to_string(),
            }
        );
    }

    #[test]
    fn serializes_modlist_entry_schema() {
        let json = serde_json::to_string(&ModListEntry {
            file_name: "example.jar".to_string(),
            sha1: "abc123".to_string(),
            source: DownloadSource::CurseForge {
                project_id: 10,
                file_id: 20,
            },
            target: super::ProjectFileTarget::Mod,
            size: 1024,
            state: ModState::Enabled,
        })
        .unwrap();

        assert_eq!(
            json,
            r#"{"fileName":"example.jar","sha1":"abc123","source":{"type":"curseForge","project_id":10,"file_id":20},"target":"mod","size":1024,"state":"enabled"}"#
        );
    }

    #[test]
    fn modlist_entry_defaults_state_to_enabled() {
        let entry: ModListEntry = serde_json::from_str(
            r#"{"fileName":"old.jar","sha1":"x","source":{"type":"manual"},"size":1}"#,
        )
        .unwrap();

        assert_eq!(entry.state, ModState::Enabled);
    }
}
