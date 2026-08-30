use std::path::{Path, PathBuf};

use reqwest::Client;
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::fetch_and_verify;

/// Builder for a single Maven artifact download. Start with [`MavenCoords::new`]
/// (repository base URL + `group:artifact:version` coordinate), optionally chain
/// [`resource_suffix`](Self::resource_suffix) / [`file_ext`](Self::file_ext),
/// then call [`download`](Self::download). Resolves to
/// `<repo>/<group as path>/<artifact>/<version>/<artifact>-<version>[-<suffix>].<ext>`.
pub struct MavenCoords<'a> {
    repo_url: &'a str,
    coords: &'a str,
    resource_suffix: Option<&'a str>,
}

impl<'a> MavenCoords<'a> {
    pub fn new(repo_url: &'a str, coords: &'a str) -> Self {
        Self { repo_url, coords, resource_suffix: None }
    }

    /// Classifier appended to the file name (e.g. `installer`).
    pub fn resource_suffix(mut self, suffix: &'a str) -> Self {
        self.resource_suffix = Some(suffix);
        self
    }

    /// `group:artifact:version` → the repo-relative path segments, joined with
    /// `/` so the same value builds both the URL and the on-disk path. A
    /// coordinate missing either colon is rejected rather than indexed into —
    /// release builds abort on panic, so a short coordinate would kill the app.
    fn coords_to_relpath(&self) -> Result<String, Error> {
        let parts: Vec<&str> = self.coords.splitn(3, ':').collect();
        let [group_id, artifact_id, version] = parts[..] else {
            return Err(Error::Invalid(format!(
                "maven coordinate '{}' is not group:artifact:version",
                self.coords
            )));
        };
        let jar_name = match self.resource_suffix {
            Some(suffix) => format!("{artifact_id}-{version}-{suffix}.jar"),
            None => format!("{artifact_id}-{version}.jar"),
        };
        Ok(format!(
            "{}/{artifact_id}/{version}/{jar_name}",
            group_id.replace('.', "/")
        ))
    }

    /// Download the artifact into `dest_path` (mirroring the repo's directory
    /// layout) and verify it against the sibling `.sha1`. Returns the path
    /// written.
    pub async fn download(self, client: &Client, dest_path: &Path) -> Result<PathBuf, Error> {
        let relative_path = self.coords_to_relpath()?;
        let url = format!("{}/{}", self.repo_url, relative_path);
        // The URL keeps `/`; only the disk path takes the platform separator.
        let file_path = relative_path
            .split('/')
            .fold(dest_path.to_path_buf(), |path, segment| path.join(segment));

        let sha1 = client.get(format!("{url}.sha1")).send().await?
            .text().await.map_err(Error::InvalidResponse)?;
        let bytes = fetch_and_verify(client, &url, sha1.trim(), self.coords).await?;

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file_path, &bytes)?;
        Ok(file_path)
    }
}
