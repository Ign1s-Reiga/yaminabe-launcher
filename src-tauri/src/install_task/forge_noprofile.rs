use std::io::Read;
use std::path::Path;
use zip::ZipArchive;
use yaminabe_launcher_shared::error::Error;
use crate::versions_dir;

pub fn install(installer_path: &Path, forge_version: &str, version_id: &str) -> Result<(), Error> {
    let jar_name = format!("forge-{forge_version}-universal.jar");
    let mut zip = ZipArchive::new(std::fs::File::open(installer_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;

    let mut jar_bytes = Vec::new();
    zip.by_name(&jar_name)
        .map_err(|e| Error::Invalid(format!("Embedded JAR not found in universal zip: {e}")))?
        .read_to_end(&mut jar_bytes)?;

    // Primary (patched) jar lives next to the version profile so every loader
    // follows the same `versions/<id>/<id>.jar` convention.
    let version_dir = versions_dir().join(version_id);
    std::fs::create_dir_all(&version_dir)?;
    std::fs::write(version_dir.join(format!("{version_id}.jar")), &jar_bytes)?;

    Ok(())
}