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
        match s {
            "forge" => Ok(ModLoader::Forge),
            "fabric" => Ok(ModLoader::Fabric),
            "quilt" => Ok(ModLoader::Quilt),
            "neoforge" => Ok(ModLoader::NeoForge),
            _ => Err(Error::Invalid(s.to_string())), // TODO: Use error kind match better with this case.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceMeta {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub mod_loader: ModLoader,
    pub mod_loader_version: Option<String>,
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
            category: String::new(),
            ram_mb: 4096,
            jvm_args: String::new(),
            jre_path: String::new(),
            description: String::new(),
            window_width: 0,
            window_height: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModpackInfo {
    pub id: u32,
    pub name: String,
    pub summary: String,
    pub logo_url: Option<String>,
    pub download_count: u32,
    pub game_versions: Vec<String>,
    pub category: Vec<String>,
    pub primary_category_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModpackSearchResults {
    pub items: Vec<ModpackInfo>,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModpackVersionFile {
    pub id: u32,
    pub mod_id: u32,
    pub release_type: String,
    pub file_name: String,
    pub download_url: String,
    pub display_name: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderVersion {
    pub id: u32,
    pub version: String,
    pub game_version: String,
    #[serde(rename = "type")]
    pub loader_type: String,
}
