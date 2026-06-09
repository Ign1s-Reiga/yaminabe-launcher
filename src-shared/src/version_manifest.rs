use std::collections::HashMap;
use serde::{Deserialize, Serialize};

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

    /// Per-side download metadata (`client`, `client_mappings`, `server`, ...).
    /// Populated by Mojang's vanilla manifest; empty for loader manifests.
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
    /// Pre-1.13 / Fabric-style libraries: the maven *repo base* URL for the
    /// bare `name` coordinate. Empty/`None` means the default Mojang repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Pre-1.13 Forge: `clientreq=false` libraries are server-only and skipped
    /// on the client side.
    #[serde(default, rename = "clientreq", skip_serializing_if = "Option::is_none")]
    pub client_req: Option<bool>,
    /// Pre-1.13 Forge: pre-computed sha1 of the artifact at `url`.
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

fn is_zero_u64(n: &u64) -> bool { *n == 0 }

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
