use std::io::Read;
use std::path::Path;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;
use yaminabe_launcher_shared::error::Error;
use crate::{libraries_dir, versions_dir};
use crate::http_utils::download_from_maven;
use super::maven_coord_to_path;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallProfile {
    install: InstallInfo,
    version_info: VersionInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallInfo {
    file_path: String,
}

#[derive(Deserialize, Serialize)]
struct VersionInfo {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<String>,
    #[serde(rename = "releaseTime", skip_serializing_if = "Option::is_none")]
    release_time: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    version_type: Option<String>,
    #[serde(rename = "inheritsFrom", skip_serializing_if = "Option::is_none")]
    inherits_from: Option<String>,
    #[serde(rename = "minecraftArguments", skip_serializing_if = "Option::is_none")]
    minecraft_arguments: Option<String>,
    #[serde(rename = "mainClass", skip_serializing_if = "Option::is_none")]
    main_class: Option<String>,
    #[serde(rename = "minimumLauncherVersion", skip_serializing_if = "Option::is_none")]
    minimum_launcher_version: Option<u32>,
    #[serde(default)]
    libraries: Vec<Library>,
}

#[derive(Deserialize, Serialize)]
struct Library {
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(default, rename = "clientreq", skip_serializing_if = "Option::is_none")]
    client_req: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checksums: Vec<String>,
}

pub async fn install(installer_path: &Path, client: &reqwest::Client) -> Result<String, Error> {
    let mut zip = ZipArchive::new(std::fs::File::open(installer_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;

    let profile: InstallProfile = {
        let mut buf = String::new();
        zip.by_name("install_profile.json")
            .map_err(|e| Error::Invalid(e.to_string()))?
            .read_to_string(&mut buf)?;
        serde_json::from_str(&buf)?
    };

    let version_id = profile.version_info.id.clone();
    let version_dir = versions_dir().join(&version_id);
    std::fs::create_dir_all(&version_dir)?;
    std::fs::write(
        version_dir.join(format!("{version_id}.json")),
        serde_json::to_string(&profile.version_info)?,
    )?;

    // Primary (patched) jar lives alongside the version profile so every loader
    // follows the same `versions/<id>/<id>.jar` convention. The manifest still
    // carries a `net.minecraftforge:forge:<ver>` library entry pointing into
    // libraries/, but that file is no longer materialized — launch.rs adds the
    // primary jar to the classpath via the version-id path instead.
    let primary_jar_path = version_dir.join(format!("{version_id}.jar"));
    if !primary_jar_path.exists() {
        let mut bytes = Vec::new();
        zip.by_name(&profile.install.file_path)
            .map_err(|e| Error::Invalid(format!("Embedded JAR '{}' not found: {e}", profile.install.file_path)))?
            .read_to_end(&mut bytes)?;
        std::fs::write(&primary_jar_path, &bytes)?;
    }

    for lib in &profile.version_info.libraries {
        if lib.url.is_empty() || lib.name.split(':').nth(3).is_some() {
            continue;
        }
        let lib_path = libraries_dir().join(maven_coord_to_path(&lib.name));
        if lib_path.exists() { continue; }
        download_from_maven(client, &lib.url, lib.name.clone(), None, "jar", libraries_dir().clone()).await?;
    }

    Ok(version_id)
}