use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;

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
    pub fn mod_loader_color(&self) -> &'static str {
        match self {
            ModLoader::Vanilla => "#406b50",
            ModLoader::Forge => "#6b5040",
            ModLoader::Fabric => "#40506b",
            ModLoader::Quilt => "#50406b",
            ModLoader::NeoForge => "#6b5b40",
        }
    }

    pub fn mod_loader_id(&self) -> Option<u32> {
        match self {
            ModLoader::Forge => Some(1),
            ModLoader::NeoForge => Some(6),
            ModLoader::Fabric => Some(4),
            ModLoader::Quilt => Some(5),
            ModLoader::Vanilla => None,
        }
    }

    /// Modrinth loader facet value (its `categories:` facet doubles as the
    /// loader filter). `None` for Vanilla, which has no loader.
    pub fn modrinth_name(&self) -> Option<&'static str> {
        match self {
            ModLoader::Forge => Some("forge"),
            ModLoader::NeoForge => Some("neoforge"),
            ModLoader::Fabric => Some("fabric"),
            ModLoader::Quilt => Some("quilt"),
            ModLoader::Vanilla => None,
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderVersion {
    pub id: u32,
    pub version: String,
    pub game_version: String,
    #[serde(rename = "type")]
    pub loader_type: String,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_time: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub version_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_launcher_version: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub main_class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Arguments>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_index: Option<AssetIndex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_version: Option<JavaVersion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<Library>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingSection>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub downloads: HashMap<String, DownloadEntry>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct JavaVersion {
    pub component: String,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Arguments {
    #[serde(default, rename = "default-user-jvm", skip_serializing_if = "Vec::is_empty")]
    pub default_user_jvm: Vec<DefaultJvmItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub game: Vec<ArgumentItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jvm: Vec<ArgumentItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArgumentItem {
    Plain(String),
    Conditional {
        #[serde(default)]
        rules: Vec<ArgRule>,
        value: ArgValue,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DefaultJvmItem {
    #[serde(default)]
    pub rules: Vec<ArgRule>,
    pub value: ArgValue,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArgRule {
    pub action: String,
    #[serde(default)]
    pub os: ArgRuleOs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<ArgFeatures>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct ArgRuleOs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, rename = "versionRange", skip_serializing_if = "Option::is_none")]
    pub version_range: Option<BuildVersionRange>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuildVersionRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct ArgFeatures {
    #[serde(default)]
    pub has_custom_resolution: bool,
    #[serde(default)]
    pub is_demo_user: bool,
    #[serde(default)]
    pub has_quick_plays_support: bool,
    #[serde(default)]
    pub is_quick_play_singleplayer: bool,
    #[serde(default)]
    pub is_quick_play_multiplayer: bool,
    #[serde(default)]
    pub is_quick_play_realms: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetIndex {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Library {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ArgRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub natives: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, rename = "clientreq", skip_serializing_if = "Option::is_none")]
    pub client_req: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checksums: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LibraryDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<LibraryArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<HashMap<String, LibraryArtifact>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LibraryArtifact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha1: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub size: u64,
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadEntry {
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha1: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<LoggingClient>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingClient {
    pub file: LoggingFile,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingFile {
    pub id: String,
    pub sha1: String,
    pub url: String,
}
