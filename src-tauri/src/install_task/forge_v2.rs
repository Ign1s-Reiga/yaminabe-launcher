use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use log::info;
use serde::Deserialize;
use zip::ZipArchive;
use yaminabe_launcher_shared::datatypes::ModLoader;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::version_manifest::{Library, ClientManifest};
use crate::{bin_dir, libraries_dir, temp_dir, versions_dir};
use crate::http_utils::{download_resource, sha1_hex};
use super::maven_coord_to_path;

#[derive(Deserialize)]
struct InstallProfileV2 {
    minecraft: String,
    path: String,
    data: HashMap<String, SidedValue>,
    processors: Vec<ProcessorSpec>,
    libraries: Vec<Library>,
}

#[derive(Deserialize)]
struct SidedValue {
    client: String,
    #[serde(default)]
    #[allow(dead_code)]
    server: String,
}

#[derive(Deserialize)]
struct ProcessorSpec {
    #[serde(default)]
    sides: Option<Vec<String>>,
    jar: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    outputs: HashMap<String, String>,
}

/// Download every library referenced by the version JSON that has a non-empty
/// download URL. Skips entries with an empty `url` (the patched
/// `forge-…:client` jar, which is produced locally by the binary patcher) and
/// entries already present on disk.
pub async fn pre_download_libraries(version_id: &str, client: &reqwest::Client) -> Result<(), Error> {
    let version_json_path = versions_dir().join(version_id).join(format!("{version_id}.json"));
    let text = std::fs::read_to_string(&version_json_path)?;
    let version = serde_json::from_str::<ClientManifest>(&text)?;

    for lib in &version.libraries {
        let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        if artifact.url.is_empty() { continue; }
        let Some(path) = artifact.path.as_deref() else { continue; };
        let dest = libraries_dir().join(path);
        if dest.exists() { continue; }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        download_resource(client, &artifact.url, dest).await?;
    }

    info!("Pre-downloaded Forge libraries for {version_id}");
    Ok(())
}

/// Native Forge / NeoForge V2 client install.
///
/// Mirrors the post-processing pipeline described in the official installer's
/// `PostProcessors` class (LGPL — referenced for the step sequence only):
///   1. Extract `install_profile.json` and `version.json` from the installer jar.
///   2. Write `version.json` to `versions/<id>/<id>.json`.
///   3. Download every library declared in `install_profile.libraries`.
///   4. Materialize the `data` map for the client side, expanding `[artifact]`
///      to a libraries-relative path, `'literal'` to the unquoted string, and
///      any other value to a path extracted from the installer jar into a
///      temp directory.
///   5. Inject the system tokens SIDE, MINECRAFT_JAR, MINECRAFT_VERSION, ROOT,
///      INSTALLER, LIBRARY_DIR.
///   6. For each processor: optionally filter by `sides`, short-circuit when
///      all declared outputs already match their expected SHA-1, otherwise
///      build the classpath, read `Main-Class` from the jar manifest,
///      substitute args (`[artifact]` or `{TOKEN}`), spawn a JVM, and verify
///      output checksums.
pub async fn install(
    loader: &ModLoader,
    installer_path: &Path,
    client: &reqwest::Client,
) -> Result<String, Error> {
    let installer_path = installer_path.to_path_buf();

    let (profile, version_json_text, version_id) = {
        let mut zip = ZipArchive::new(std::fs::File::open(&installer_path)?)
            .map_err(|e| Error::Invalid(e.to_string()))?;

        let mut profile_buf = String::new();
        zip.by_name("install_profile.json")
            .map_err(|e| Error::Invalid(format!("install_profile.json missing: {e}")))?
            .read_to_string(&mut profile_buf)?;
        let profile: InstallProfileV2 = serde_json::from_str(&profile_buf)?;

        let mut version_buf = String::new();
        zip.by_name("version.json")
            .map_err(|e| Error::Invalid(format!("version.json missing: {e}")))?
            .read_to_string(&mut version_buf)?;
        let parsed: ClientManifest = serde_json::from_str(&version_buf)?;
        let version_id = parsed.id.clone()
            .ok_or_else(|| Error::Invalid("V2 installer version.json missing top-level `id`".to_string()))?;

        (profile, version_buf, version_id)
    };

    // Empty `data` + empty `processors` means the installer ships a pre-built
    // universal jar (e.g. 1.12.2-2864) and runtime FMLTweaker handles the
    // binpatching in-memory — there is nothing for the V2 processor pipeline
    // to do. The remaining work (write manifest, place primary jar) is
    // identical to the V1 path, so delegate.
    if profile.data.is_empty() && profile.processors.is_empty() {
        let universal = profile.libraries.iter()
            .find(|lib| lib.name == profile.path)
            .ok_or_else(|| Error::Invalid(format!("install_profile.libraries has no entry matching path {}", profile.path)))?;
        let artifact = universal.downloads.as_ref().and_then(|d| d.artifact.as_ref())
            .ok_or_else(|| Error::Invalid(format!("Universal library {} has no artifact downloads", profile.path)))?;
        let artifact_path = artifact.path.as_deref()
            .ok_or_else(|| Error::Invalid(format!("Universal library {} artifact has no path", profile.path)))?;

        let embedded = format!("maven/{artifact_path}");
        let mut zip = ZipArchive::new(std::fs::File::open(&installer_path)?)
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let mut primary_jar_bytes = Vec::new();
        zip.by_name(&embedded)
            .map_err(|e| Error::Invalid(format!("Embedded jar '{embedded}' not found: {e}")))?
            .read_to_end(&mut primary_jar_bytes)?;
        if !artifact.sha1.is_empty() {
            let actual = sha1_hex(&primary_jar_bytes);
            if actual != artifact.sha1 {
                return Err(Error::ChecksumMismatch {
                    resource: format!("embedded {embedded}"),
                    sha1: artifact.sha1.clone(),
                    hex: actual,
                });
            }
        }

        let version_info: ClientManifest = serde_json::from_str(&version_json_text)?;
        let v1_version_id = super::forge_v1::install_from_parsed(&version_info, &version_json_text, &profile.path, &primary_jar_bytes, client).await?;
        info!("Installed {loader} via V1 delegation ({})", installer_path.display());
        return Ok(v1_version_id);
    }

    let version_dir = versions_dir().join(&version_id);
    std::fs::create_dir_all(&version_dir)?;
    std::fs::write(version_dir.join(format!("{version_id}.json")), &version_json_text)?;

    // The installer jar embeds a `maven/<path>` tree of libraries that are
    // produced by Forge itself (e.g. the patched `forge-<ver>.jar` for 1.12.2)
    // and are not actually downloadable from the URL in `install_profile.json`.
    // Match the official installer's order: extract from `maven/...` first,
    // fall back to URL download, only then give up if both are absent.
    for lib in &profile.libraries {
        let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        let Some(artifact_path) = artifact.path.as_deref() else { continue; };
        let dest = libraries_dir().join(artifact_path);
        if dest.exists() { continue; }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let extracted = {
            let mut zip = ZipArchive::new(std::fs::File::open(&installer_path)?)
                .map_err(|e| Error::Invalid(e.to_string()))?;
            let embedded = format!("maven/{artifact_path}");
            match zip.by_name(&embedded) {
                Ok(mut entry) => {
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf)?;
                    if !artifact.sha1.is_empty() {
                        let actual = sha1_hex(&buf);
                        if actual != artifact.sha1 {
                            return Err(Error::ChecksumMismatch {
                                resource: format!("embedded {embedded}"),
                                sha1: artifact.sha1.clone(),
                                hex: actual,
                            });
                        }
                    }
                    std::fs::write(&dest, &buf)?;
                    true
                }
                Err(_) => false,
            }
        };

        if !extracted {
            if artifact.url.is_empty() { continue; }
            download_resource(client, &artifact.url, dest.clone()).await?;
            if !artifact.sha1.is_empty() {
                let actual = sha1_hex(&std::fs::read(&dest)?);
                if actual != artifact.sha1 {
                    std::fs::remove_file(&dest).ok();
                    return Err(Error::ChecksumMismatch {
                        resource: artifact.url.clone(),
                        sha1: artifact.sha1.clone(),
                        hex: actual,
                    });
                }
            }
        }
    }

    let extract_dir = temp_dir().join(format!("forge_v2_{version_id}"));
    std::fs::create_dir_all(&extract_dir)?;
    let mut data: HashMap<String, String> = HashMap::new();
    {
        let mut zip = ZipArchive::new(std::fs::File::open(&installer_path)?)
            .map_err(|e| Error::Invalid(e.to_string()))?;
        for (key, val) in &profile.data {
            data.insert(key.clone(), resolve_data_entry(&val.client, &mut zip, &extract_dir)?);
        }
    }

    let mc_jar = versions_dir()
        .join(&profile.minecraft)
        .join(format!("{}.jar", profile.minecraft));
    data.insert("SIDE".into(), "client".into());
    data.insert("MINECRAFT_JAR".into(), mc_jar.to_string_lossy().into_owned());
    data.insert("MINECRAFT_VERSION".into(), profile.minecraft.clone());
    data.insert("ROOT".into(), bin_dir().to_string_lossy().into_owned());
    data.insert("INSTALLER".into(), installer_path.to_string_lossy().into_owned());
    data.insert("LIBRARY_DIR".into(), libraries_dir().to_string_lossy().into_owned());

    let total = profile.processors.len();
    for (idx, proc) in profile.processors.iter().enumerate() {
        if let Some(sides) = &proc.sides {
            if !sides.iter().any(|s| s == "client") { continue; }
        }

        let outputs: Vec<(PathBuf, String)> = proc.outputs.iter()
            .map(|(k, v)| (PathBuf::from(resolve_arg(k, &data)), replace_tokens(v, &data)))
            .collect();

        if !outputs.is_empty() && outputs_satisfied(&outputs)? {
            info!("[{loader}] processor {}/{total} cache hit", idx + 1);
            continue;
        }

        let jar_path = libraries_dir().join(maven_coord_to_path(&proc.jar));
        if !jar_path.is_file() {
            return Err(Error::Invalid(format!("Missing processor jar: {}", jar_path.display())));
        }
        let main_class = read_main_class(&jar_path)?;

        let cp_sep = if cfg!(windows) { ";" } else { ":" };
        let mut cp_entries: Vec<String> = Vec::with_capacity(proc.classpath.len() + 1);
        cp_entries.push(jar_path.to_string_lossy().into_owned());
        for dep in &proc.classpath {
            let dep_path = libraries_dir().join(maven_coord_to_path(dep));
            if !dep_path.is_file() {
                return Err(Error::Invalid(format!("Missing processor dependency: {}", dep_path.display())));
            }
            cp_entries.push(dep_path.to_string_lossy().into_owned());
        }
        let classpath = cp_entries.join(cp_sep);

        let args: Vec<String> = proc.args.iter().map(|a| resolve_arg(a, &data)).collect();

        info!("[{loader}] processor {}/{total}: {main_class}", idx + 1);

        let mut cmd = tokio::process::Command::new("java");
        cmd.arg("-cp").arg(&classpath).arg(&main_class).args(&args);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);

        let status = cmd.status().await
            .map_err(|e| Error::ChildProcess(format!("[{loader}] processor: {e}")))?;
        if !status.success() {
            return Err(Error::ChildProcess(format!("[{loader}] processor {} exited with {status}", idx + 1)));
        }

        for (path, expected) in &outputs {
            if !path.exists() {
                return Err(Error::Invalid(format!("Processor output missing: {}", path.display())));
            }
            let actual = sha1_hex(&std::fs::read(path)?);
            if actual != *expected {
                std::fs::remove_file(path).ok();
                return Err(Error::ChecksumMismatch {
                    resource: path.to_string_lossy().into_owned(),
                    sha1: expected.clone(),
                    hex: actual,
                });
            }
        }
    }

    info!("Installed {loader} via native processors ({})", installer_path.display());
    Ok(version_id)
}

fn outputs_satisfied(outputs: &[(PathBuf, String)]) -> Result<bool, Error> {
    for (path, expected) in outputs {
        if !path.exists() { return Ok(false); }
        let actual = sha1_hex(&std::fs::read(path)?);
        if actual != *expected {
            std::fs::remove_file(path).ok();
            return Ok(false);
        }
    }
    Ok(true)
}

fn resolve_data_entry(
    raw: &str,
    zip: &mut ZipArchive<std::fs::File>,
    extract_dir: &Path,
) -> Result<String, Error> {
    let bytes = raw.as_bytes();
    if bytes.first() == Some(&b'[') && bytes.last() == Some(&b']') {
        let coord = &raw[1..raw.len() - 1];
        Ok(libraries_dir().join(maven_coord_to_path(coord)).to_string_lossy().into_owned())
    } else if bytes.first() == Some(&b'\'') && bytes.last() == Some(&b'\'') {
        Ok(raw[1..raw.len() - 1].to_string())
    } else {
        let stripped = raw.trim_start_matches('/');
        let dest = extract_dir.join(stripped);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut entry = zip.by_name(stripped)
            .map_err(|e| Error::Invalid(format!("Installer resource '{stripped}' missing: {e}")))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&dest, &buf)?;
        Ok(dest.to_string_lossy().into_owned())
    }
}

fn resolve_arg(arg: &str, data: &HashMap<String, String>) -> String {
    let bytes = arg.as_bytes();
    if bytes.first() == Some(&b'[') && bytes.last() == Some(&b']') {
        let coord = &arg[1..arg.len() - 1];
        libraries_dir().join(maven_coord_to_path(coord)).to_string_lossy().into_owned()
    } else {
        replace_tokens(arg, data)
    }
}

fn replace_tokens(s: &str, data: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        match rest.find('{') {
            Some(start) => {
                out.push_str(&rest[..start]);
                let after = &rest[start..];
                if let Some(end) = after.find('}') {
                    let key = &after[1..end];
                    if let Some(val) = data.get(key) {
                        out.push_str(val);
                        rest = &after[end + 1..];
                        continue;
                    }
                }
                out.push('{');
                rest = &after[1..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

fn read_main_class(jar_path: &Path) -> Result<String, Error> {
    let mut zip = ZipArchive::new(std::fs::File::open(jar_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let mut text = String::new();
    zip.by_name("META-INF/MANIFEST.MF")
        .map_err(|e| Error::Invalid(format!("Jar missing manifest: {e}")))?
        .read_to_string(&mut text)?;

    // Manifest line continuations: a leading space on the next line is a continuation
    // of the previous logical line. Unfold before scanning.
    let unfolded = text.replace("\r\n ", "").replace("\n ", "").replace("\r ", "");
    for line in unfolded.lines() {
        if let Some(rest) = line.strip_prefix("Main-Class:") {
            return Ok(rest.trim().to_string());
        }
    }
    Err(Error::Invalid(format!("Jar manifest has no Main-Class: {}", jar_path.display())))
}