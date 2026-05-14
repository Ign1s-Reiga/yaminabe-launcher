use std::io::Read;
use std::path::Path;
use zip::ZipArchive;
use yaminabe_launcher_shared::error::Error;
use crate::libraries_dir;

pub fn install(installer_path: &Path, forge_version: &str) -> Result<(), Error> {
    let jar_name = format!("forge-{forge_version}-universal.jar");
    let mut zip = ZipArchive::new(std::fs::File::open(installer_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;

    let mut jar_bytes = Vec::new();
    zip.by_name(&jar_name)
        .map_err(|e| Error::Invalid(format!("Embedded JAR not found in universal zip: {e}")))?
        .read_to_end(&mut jar_bytes)?;

    let lib_path = libraries_dir()
        .join("net").join("minecraftforge").join("forge")
        .join(forge_version)
        .join(&jar_name);

    if let Some(parent) = lib_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&lib_path, &jar_bytes)?;

    Ok(())
}