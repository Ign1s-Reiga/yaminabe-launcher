use serde::Deserialize;
use std::io::Read;
use std::str::FromStr;
use std::path::Path;
use yaminabe_launcher_shared::datamodels::{LocalModpackInfo, ModLoader, ModpackFormat};
use yaminabe_launcher_shared::error::Error;

#[derive(Debug, Deserialize)]
pub struct ModpackManifest {
    pub minecraft: ManifestVersionSpecifier,
    #[serde(default = "default_overrides_dir")]
    pub overrides: String,
    #[serde(default)]
    pub files: Vec<ManifestFilesItem>,
    /// Shown before installing a pack picked off disk. Optional, since nothing
    /// in the install itself depends on what the pack calls itself.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Deserialize)]
pub struct ManifestFilesItem {
    #[serde(rename = "projectID")]
    pub project_id: u32,
    #[serde(rename = "fileID")]
    pub file_id: u32,
    pub required: bool,
}

fn default_overrides_dir() -> String {
    "overrides".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestVersionSpecifier {
    pub version: String,
    #[serde(default)]
    pub mod_loaders: Vec<ModLoaderVersion>,
}

#[derive(Debug, Deserialize)]
pub struct ModLoaderVersion {
    pub id: String,
    pub primary: bool,
}


pub fn read_manifest<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<ModpackManifest, Error> {
    let mut file = archive
        .by_name("manifest.json")
        .map_err(|_| Error::Invalid("modpack zip is missing manifest.json".to_string()))?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(serde_json::from_str(&content)?)
}

/// Resolve the modpack's primary mod loader and its version. CurseForge
/// manifest loader ids are `{name}-{version}` (e.g. `neoforge-21.1.228`).
pub fn resolve_loader(manifest: &ModpackManifest) -> Result<(ModLoader, Option<String>), Error> {
    let mod_loader_id = manifest
        .minecraft
        .mod_loaders
        .iter()
        .find(|l| l.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first())
        .map(|l| l.id.to_ascii_lowercase())
        .unwrap_or("vanilla".to_string());
    let (loader_name, loader_version) = mod_loader_id
        .split_once('-')
        .map(|(n, v)| (n, Some(v.to_string())))
        .unwrap_or((mod_loader_id.as_str(), None));
    Ok((ModLoader::from_str(loader_name)?, loader_version))
}

pub fn manifest_file_ids(manifest: &ModpackManifest) -> Vec<u32> {
    manifest
        .files
        .iter()
        .filter(|f| f.required)
        .map(|f| f.file_id)
        .collect()
}


/// Read a modpack zip's manifest without installing anything, so the user can
/// confirm the file they picked and a zip that is not a CurseForge modpack is
/// rejected before an instance directory exists.
pub fn read_local_modpack(zip_path: &Path) -> Result<LocalModpackInfo, Error> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| Error::Invalid(format!("cannot open {}: {e}", zip_path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| Error::Invalid(format!("{} is not a zip file", zip_path.display())))?;
    let manifest = read_manifest(&mut archive)?;
    let (mod_loader, mod_loader_version) = resolve_loader(&manifest)?;
    Ok(LocalModpackInfo {
        format: ModpackFormat::CurseForge,
        name: manifest.name,
        version: manifest.version,
        author: manifest.author,
        game_version: manifest.minecraft.version,
        mod_loader,
        mod_loader_version,
        file_count: manifest.files.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::read_local_modpack;
    use std::io::Write;
    use yaminabe_launcher_shared::datamodels::ModLoader;

    /// Write a zip holding exactly `entries`, and return where it landed.
    fn zip_with(name: &str, entries: &[(&str, &str)]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("yaminabe-test-{name}.zip"));
        let file = std::fs::File::create(&path).expect("create test zip");
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (entry, body) in entries {
            writer.start_file(*entry, options).expect("start entry");
            writer.write_all(body.as_bytes()).expect("write entry");
        }
        writer.finish().expect("finish zip");
        path
    }

    /// Shaped after a real CurseForge export, field for field.
    const MANIFEST: &str = r#"{
        "minecraft": {
            "version": "1.21.1",
            "modLoaders": [{ "id": "neoforge-21.1.248", "primary": true }]
        },
        "manifestType": "minecraftModpack",
        "manifestVersion": 1,
        "name": "FTB StoneBlock 4",
        "version": "1.20.0",
        "author": "FTB Team",
        "files": [
            { "projectID": 1, "fileID": 2, "required": true },
            { "projectID": 3, "fileID": 4, "required": false }
        ],
        "overrides": "overrides"
    }"#;

    #[test]
    fn reads_what_a_curseforge_manifest_declares() {
        let path = zip_with("manifest", &[("manifest.json", MANIFEST)]);

        let info = read_local_modpack(&path).expect("manifest should parse");

        assert_eq!(info.name, "FTB StoneBlock 4");
        assert_eq!(info.version, "1.20.0");
        assert_eq!(info.author, "FTB Team");
        assert_eq!(info.game_version, "1.21.1");
        assert_eq!(info.mod_loader, ModLoader::NeoForge);
        assert_eq!(info.mod_loader_version.as_deref(), Some("21.1.248"));
        // Optional files count too: the picker reports the manifest, not the
        // subset that will be downloaded.
        assert_eq!(info.file_count, 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_a_zip_that_is_not_a_modpack() {
        let path = zip_with("no-manifest", &[("readme.txt", "just a zip")]);

        let error = read_local_modpack(&path).expect_err("a zip with no manifest is not a modpack");

        assert!(
            error.to_string().contains("manifest.json"),
            "the error should name what is missing, got: {error}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_a_file_that_is_not_a_zip() {
        let path = std::env::temp_dir().join("yaminabe-test-not-a-zip.zip");
        std::fs::write(&path, b"not a zip at all").expect("write test file");

        let error = read_local_modpack(&path).expect_err("plain bytes are not a zip");

        assert!(error.to_string().contains("not a zip"), "got: {error}");
        std::fs::remove_file(&path).ok();
    }
}
