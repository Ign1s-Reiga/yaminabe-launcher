use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ModTool {
    Vanilla,
    Forge,
    Fabric,
    Quilt,
    NeoForge,
}

impl fmt::Display for ModTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ModTool::Vanilla => "Vanilla",
            ModTool::Forge => "Forge",
            ModTool::Fabric => "Fabric",
            ModTool::Quilt => "Quilt",
            ModTool::NeoForge => "NeoForge",
        })
    }
}

impl FromStr for ModTool {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "forge" => Ok(ModTool::Forge),
            "fabric" => Ok(ModTool::Fabric),
            "quilt" => Ok(ModTool::Quilt),
            "neoforge" => Ok(ModTool::NeoForge),
            _ => Ok(ModTool::Vanilla),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceMeta {
    pub id: String,
    pub name: String,
    pub mc_version: String,
    pub mod_tool: String,
    pub mod_tool_version: Option<String>,
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
            jvm_args:           String::new(),
            memory_mb:          4096,
            instance_install_dir: String::new(),
            window_width:       0,
            window_height:      0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaInstall {
    pub path: String,
    pub version: String,
    pub vendor: String,
}
