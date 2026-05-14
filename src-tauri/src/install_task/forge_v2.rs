use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use log::info;
use serde::Deserialize;
use zip::ZipArchive;
use yaminabe_launcher_shared::datatypes::ModLoader;
use yaminabe_launcher_shared::error::Error;
use crate::{bin_dir, libraries_dir, temp_dir, versions_dir};
use crate::http_utils::download_resource;
use super::{maven_coord_to_path, sha1_hex};

#[derive(Deserialize)]
struct VersionJson {
    libraries: Vec<VersionLibrary>,
}

#[derive(Deserialize)]
struct VersionLibrary {
    #[serde(default)]
    downloads: Option<VersionLibraryDownloads>,
}

#[derive(Deserialize)]
struct VersionLibraryDownloads {
    artifact: Option<VersionLibraryArtifact>,
}

#[derive(Deserialize)]
struct VersionLibraryArtifact {
    path: String,
    url: String,
}

/// Download every library referenced by the version JSON that has a non-empty
/// download URL. Skips entries with an empty `url` (the patched
/// `forge-…:client` jar, which is produced locally by the binary patcher) and
/// entries already present on disk.
pub async fn pre_download_libraries(version_id: &str, client: &reqwest::Client) -> Result<(), Error> {
    let version_json_path = versions_dir().join(version_id).join(format!("{version_id}.json"));
    let text = std::fs::read_to_string(&version_json_path)?;
    let version = serde_json::from_str::<VersionJson>(&text)?;

    for lib in &version.libraries {
        let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        if artifact.url.is_empty() { continue; }
        let dest = libraries_dir().join(&artifact.path);
        if dest.exists() { continue; }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        download_resource(client, &artifact.url, dest).await?;
    }

    info!("Pre-downloaded Forge libraries for {version_id}");
    Ok(())
}

#[derive(Deserialize)]
struct InstallProfile {
    version: String,
    #[serde(default)]
    minecraft: String,
    libraries: Vec<InstallLibrary>,
    processors: Vec<InstallProcessor>,
    data: HashMap<String, InstallDataEntry>,
}

#[derive(Deserialize)]
struct InstallLibrary {
    name: String,
    downloads: InstallLibraryDownloads,
}

#[derive(Deserialize)]
struct InstallLibraryDownloads {
    artifact: InstallArtifact,
}

#[derive(Deserialize)]
struct InstallArtifact {
    path: String,
    url: String,
    sha1: String,
}

#[derive(Deserialize)]
struct InstallProcessor {
    jar: String,
    classpath: Vec<String>,
    args: Vec<String>,
    #[serde(default)]
    sides: Vec<String>,
}

#[derive(Deserialize)]
struct InstallDataEntry {
    client: String,
}

fn read_jar_main_class(jar_path: &Path) -> Result<String, Error> {
    let mut zip = ZipArchive::new(std::fs::File::open(jar_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let mut content = String::new();
    zip.by_name("META-INF/MANIFEST.MF")
        .map_err(|e| Error::Invalid(e.to_string()))?
        .read_to_string(&mut content)?;
    content.lines()
        .find_map(|line| line.strip_prefix("Main-Class:").map(|s| s.trim().to_owned()))
        .ok_or_else(|| Error::Invalid(format!("Main-Class not found in {}", jar_path.display())))
}

fn coord_to_abs_path(coord: &str) -> String {
    libraries_dir().join(maven_coord_to_path(coord)).to_string_lossy().into_owned()
}

fn unwrap_brackets(s: &str, open: char, close: char) -> Option<&str> {
    s.strip_prefix(open).and_then(|rest| rest.strip_suffix(close))
}

struct ResolveCtx<'a> {
    minecraft_version: &'a str,
    installer_path: &'a Path,
    extracted_data: &'a HashMap<String, PathBuf>,
}

fn resolve_proc_arg(
    arg: &str,
    data: &HashMap<String, InstallDataEntry>,
    ctx: &ResolveCtx,
) -> String {
    if let Some(key) = unwrap_brackets(arg, '{', '}') {
        // Built-in placeholders defined by the Forge installer spec.
        match key {
            "SIDE" => return "client".to_owned(),
            "MINECRAFT_VERSION" => return ctx.minecraft_version.to_owned(),
            "MINECRAFT_JAR" => return versions_dir()
                .join(ctx.minecraft_version)
                .join(format!("{}.jar", ctx.minecraft_version))
                .to_string_lossy().into_owned(),
            "INSTALLER" => return ctx.installer_path.to_string_lossy().into_owned(),
            "LIBRARY_DIR" => return libraries_dir().to_string_lossy().into_owned(),
            "ROOT" => return bin_dir().to_string_lossy().into_owned(),
            _ => {}
        }
        // Data entries whose value was a path inside the installer jar — already
        // extracted to a temporary file on disk.
        if let Some(path) = ctx.extracted_data.get(key) {
            return path.to_string_lossy().into_owned();
        }
        let Some(entry) = data.get(key) else {
            return arg.to_owned();
        };
        if let Some(coord) = unwrap_brackets(&entry.client, '[', ']') {
            return coord_to_abs_path(coord);
        }
        if let Some(lit) = unwrap_brackets(&entry.client, '\'', '\'') {
            return lit.to_owned();
        }
        return entry.client.clone();
    }
    if let Some(coord) = unwrap_brackets(arg, '[', ']') {
        return coord_to_abs_path(coord);
    }
    arg.to_owned()
}

/// Extract any data entries whose `client` value is a path inside the installer
/// jar (e.g. `/data/client.lzma`) into a temp directory. Returns a map from the
/// data key to the on-disk path of the extracted file.
fn extract_jar_data_entries(
    zip: &mut ZipArchive<std::fs::File>,
    data: &HashMap<String, InstallDataEntry>,
) -> Result<HashMap<String, PathBuf>, Error> {
    let extract_dir = temp_dir().join("forge-install-data");
    std::fs::create_dir_all(&extract_dir)?;
    let mut out: HashMap<String, PathBuf> = HashMap::new();
    for (key, entry) in data {
        if !entry.client.starts_with('/') { continue; }
        let inner = entry.client.trim_start_matches('/');
        let mut zip_entry = zip.by_name(inner)
            .map_err(|e| Error::Invalid(format!("missing {inner} in installer: {e}")))?;
        let file_name = Path::new(inner).file_name()
            .ok_or_else(|| Error::Invalid(format!("bad jar path: {inner}")))?;
        let dest = extract_dir.join(file_name);
        let mut bytes = Vec::new();
        zip_entry.read_to_end(&mut bytes)?;
        std::fs::write(&dest, &bytes)?;
        out.insert(key.clone(), dest);
    }
    Ok(out)
}

pub async fn install(
    loader: &ModLoader,
    installer_path: &Path,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let mut zip = ZipArchive::new(std::fs::File::open(installer_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;

    let profile: InstallProfile = {
        let mut buf = String::new();
        zip.by_name("install_profile.json")
            .map_err(|e| Error::Invalid(e.to_string()))?
            .read_to_string(&mut buf)?;
        serde_json::from_str(&buf)?
    };
    let version_json = {
        let mut buf = String::new();
        zip.by_name("version.json")
            .map_err(|e| Error::Invalid(e.to_string()))?
            .read_to_string(&mut buf)?;
        buf
    };

    info!("Running {loader} install profile: {}", profile.version);

    let minecraft_version = if profile.minecraft.is_empty() {
        // Fall back to the prefix of e.g. "1.20.1-forge-47.2.0".
        profile.version.split('-').next().unwrap_or(&profile.version).to_owned()
    } else {
        profile.minecraft.clone()
    };

    let version_dir = versions_dir().join(&profile.version);
    std::fs::create_dir_all(&version_dir)?;
    std::fs::write(version_dir.join(format!("{}.json", profile.version)), &version_json)?;

    let extracted_data = extract_jar_data_entries(&mut zip, &profile.data)?;

    for lib in &profile.libraries {
        let lib_path = libraries_dir().join(&lib.downloads.artifact.path);
        if lib_path.exists() || lib.downloads.artifact.url.is_empty() {
            continue;
        }
        if let Some(parent) = lib_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = client.get(&lib.downloads.artifact.url).send().await?
            .bytes().await.map_err(Error::InvalidResponse)?;
        let hex = sha1_hex(&bytes);
        if hex != lib.downloads.artifact.sha1 {
            return Err(Error::ChecksumMismatch {
                resource: lib.name.clone(),
                sha1: lib.downloads.artifact.sha1.clone(),
                hex,
            });
        }
        std::fs::write(&lib_path, &bytes)?;
    }

    let ctx = ResolveCtx {
        minecraft_version: &minecraft_version,
        installer_path,
        extracted_data: &extracted_data,
    };
    let cp_sep = if cfg!(windows) { ";" } else { ":" };
    for processor in &profile.processors {
        if !processor.sides.is_empty() && !processor.sides.iter().any(|s| s == "client") {
            continue;
        }
        let proc_jar = libraries_dir().join(maven_coord_to_path(&processor.jar));
        let main_class = read_jar_main_class(&proc_jar)?;
        let classpath = std::iter::once(proc_jar.to_string_lossy().into_owned())
            .chain(processor.classpath.iter().map(|cp| coord_to_abs_path(cp)))
            .collect::<Vec<_>>()
            .join(cp_sep);
        let args: Vec<String> = processor.args.iter()
            .map(|a| resolve_proc_arg(a, &profile.data, &ctx))
            .collect();
        let status = tokio::process::Command::new("java")
            .args(["-cp", &classpath, &main_class])
            .args(&args)
            .status().await
            .map_err(|e| Error::ChildProcess(format!("[{loader}] {}: {e}", processor.jar)))?;
        if !status.success() {
            return Err(Error::ChildProcess(format!("[{loader}] {} exited with {status}", processor.jar)));
        }
    }

    Ok(())
}
