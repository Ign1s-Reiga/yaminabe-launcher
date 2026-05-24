//! Helpers for reading single entries out of a Forge/NeoForge installer jar.
//!
//! Every callsite that wants one file out of an installer used to open the
//! file, wrap it in a `ZipArchive`, look up `by_name`, and read the contents
//! — repeated four ways across `mod.rs`, `forge_v1.rs`, and `forge_v2.rs`.
//! The helpers below collapse that into one call; the `Error::Invalid`
//! mapping is uniform.
//!
//! Each call opens the installer fresh. For paths that need many entries
//! from the same archive (notably `forge_v2::install`), open the
//! `ZipArchive` directly so the file isn't read off disk N times.

use std::io::Read;
use std::path::Path;
use zip::ZipArchive;
use yaminabe_launcher_shared::error::Error;

/// Open an installer jar and return its `ZipArchive`. Surfaces both IO and
/// zip-format failures as typed errors with the installer path included.
pub fn open(installer_path: &Path) -> Result<ZipArchive<std::fs::File>, Error> {
    let file = std::fs::File::open(installer_path)?;
    ZipArchive::new(file).map_err(|e| Error::Invalid(format!(
        "could not read installer {} as a zip: {e}",
        installer_path.display()
    )))
}

/// Read a single entry's bytes from an installer jar. Errors include the
/// installer path and the missing entry name for diagnosability.
pub fn read_entry_bytes(installer_path: &Path, entry_name: &str) -> Result<Vec<u8>, Error> {
    let mut zip = open(installer_path)?;
    let mut entry = zip.by_name(entry_name).map_err(|e| Error::Invalid(format!(
        "installer {} has no entry '{entry_name}': {e}",
        installer_path.display()
    )))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Read a single entry's contents from an installer jar as a UTF-8 string.
pub fn read_entry_str(installer_path: &Path, entry_name: &str) -> Result<String, Error> {
    let bytes = read_entry_bytes(installer_path, entry_name)?;
    String::from_utf8(bytes).map_err(|e| Error::Invalid(format!(
        "installer {} entry '{entry_name}' is not valid UTF-8: {e}",
        installer_path.display()
    )))
}