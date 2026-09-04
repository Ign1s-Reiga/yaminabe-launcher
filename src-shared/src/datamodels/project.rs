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

    /// The file half of this source as a string, so a version picker can carry
    /// a pick through a form without knowing which platform listed it. `None`
    /// for a hand-added file, which no version list offers.
    pub fn version_key(&self) -> Option<String> {
        match self {
            DownloadSource::CurseForge { file_id, .. } => Some(file_id.to_string()),
            DownloadSource::Modrinth { version_id, .. } => Some(version_id.clone()),
            DownloadSource::Manual => None,
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
    ShaderPack,
    World,
    DataPack,
    Modpack,
}

impl ProjectFileTarget {
    pub fn directory(&self) -> &'static str {
        match self {
            ProjectFileTarget::Mod => "mods",
            ProjectFileTarget::ResourcePack => "resourcepacks",
            ProjectFileTarget::ShaderPack => "shaderpacks",
            ProjectFileTarget::World => "saves",
            ProjectFileTarget::DataPack => "datapacks",
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
            ProjectFileTarget::ShaderPack => "6552",
            ProjectFileTarget::World => "17",
            ProjectFileTarget::DataPack => "6945",
            ProjectFileTarget::Modpack => "4471",
        }
    }

    pub fn to_modrinth_type(&self) -> &'static str {
        match self {
            ProjectFileTarget::Mod => "mod",
            ProjectFileTarget::ResourcePack => "resourcepack",
            ProjectFileTarget::ShaderPack => "shader",
            ProjectFileTarget::World => "world",
            ProjectFileTarget::DataPack => "datapack",
            ProjectFileTarget::Modpack => "modpack",
        }
    }

    pub fn from_curseforge_type(project_type: u32) -> ProjectFileTarget {
        match project_type {
            12 => ProjectFileTarget::ResourcePack,
            17 => ProjectFileTarget::World,
            4471 => ProjectFileTarget::Modpack,
            6552 => ProjectFileTarget::ShaderPack,
            6945 => ProjectFileTarget::DataPack,
            _ => ProjectFileTarget::Mod,
        }
    }

    pub fn from_modrinth_type(project_type: &str) -> ProjectFileTarget {
        match project_type {
            "resourcepack" => ProjectFileTarget::ResourcePack,
            "shader" => ProjectFileTarget::ShaderPack,
            "world" => ProjectFileTarget::World,
            "datapack" => ProjectFileTarget::DataPack,
            "modpack" => ProjectFileTarget::Modpack,
            _ => ProjectFileTarget::Mod,
        }
    }
}

/// Which project a search result names, on whichever platform it came from.
///
/// The two platforms identify projects differently — CurseForge numerically,
/// Modrinth by an opaque string — so the id carries its platform rather than
/// sitting beside a separate field that could disagree with it. This mirrors
/// how `DownloadSource` already splits a file's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "platform", content = "id")]
pub enum ProjectId {
    CurseForge(u32),
    Modrinth(String),
}

impl ProjectId {
    pub fn platform(&self) -> Platform {
        match self {
            ProjectId::CurseForge(_) => Platform::CurseForge,
            ProjectId::Modrinth(_) => Platform::Modrinth,
        }
    }

    /// The numeric id, for the CurseForge API. `None` for any other platform,
    /// so a Modrinth project cannot reach an endpoint that cannot serve it.
    pub fn curseforge(&self) -> Option<u32> {
        match self {
            ProjectId::CurseForge(id) => Some(*id),
            _ => None,
        }
    }

    /// The string id, for the Modrinth API.
    pub fn modrinth(&self) -> Option<&str> {
        match self {
            ProjectId::Modrinth(id) => Some(id),
            _ => None,
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
    /// Names the project and the platform it lives on together.
    pub id: ProjectId,
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

/// Which modpack format a file on disk turned out to be. They differ in more
/// than layout: a CurseForge manifest names mods by project id and needs the
/// API to resolve them, while a Modrinth index carries download URLs and hashes
/// outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModpackFormat {
    /// `manifest.json` at the root of a `.zip`.
    CurseForge,
    /// `modrinth.index.json` at the root of a `.mrpack`.
    Modrinth,
}

impl Display for ModpackFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModpackFormat::CurseForge => write!(f, "CurseForge"),
            ModpackFormat::Modrinth => write!(f, "Modrinth"),
        }
    }
}

/// What a modpack file on disk says about itself, read before installing so the
/// user can confirm they picked the right pack and a file that is not a modpack
/// at all is rejected up front rather than mid-install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModpackInfo {
    pub format: ModpackFormat,
    /// The pack's own name from its manifest; empty when it declares none.
    pub name: String,
    pub version: String,
    pub author: String,
    pub game_version: String,
    pub mod_loader: ModLoader,
    pub mod_loader_version: Option<String>,
    /// How many files the manifest lists, required and optional together.
    pub file_count: usize,
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
    /// The project's name on the distribution site, empty when unknown.
    #[serde(default)]
    pub project_name: String,
    /// The project's icon on the distribution site, if it publishes one.
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
}

impl ProjectFileInfo {
    pub fn to_modlist_entry(&self, state: ModState) -> ModListEntry {
        ModListEntry {
            file_name: self.file_name.clone(),
            project_name: self.project_name.clone(),
            icon_url: self.icon_url.clone(),
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
    /// The project's name on the distribution site it was downloaded from.
    /// Empty for a hand-added mod, and for entries written before it was recorded.
    #[serde(default)]
    pub project_name: String,
    /// The project's icon on that site, when it publishes one.
    #[serde(default)]
    pub icon_url: Option<String>,
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

impl ModListEntry {
    /// What a mod list shows for this entry: the name the distribution site uses,
    /// falling back to the file name for a hand-added mod.
    pub fn display_name(&self) -> &str {
        if self.project_name.is_empty() {
            &self.file_name
        } else {
            &self.project_name
        }
    }
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
            project_name: "Example Mod".to_string(),
            icon_url: Some("https://example.invalid/icon.png".to_string()),
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
            r#"{"fileName":"example.jar","projectName":"Example Mod","iconUrl":"https://example.invalid/icon.png","sha1":"abc123","source":{"type":"curseForge","project_id":10,"file_id":20},"target":"mod","size":1024,"state":"enabled"}"#
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
