use super::minecraft::ModLoader;
use super::project::DownloadSource;
use serde::{Deserialize, Serialize};

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
pub struct AccountSummary {
    pub uuid: String,
    pub username: String,
}

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
