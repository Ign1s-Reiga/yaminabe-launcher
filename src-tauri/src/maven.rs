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
    file_ext: Option<&'a str>,
}

impl<'a> MavenCoords<'a> {
    pub fn new(repo_url: &'a str, coords: &'a str) -> Self {
        Self { repo_url, coords, resource_suffix: None, file_ext: None }
    }

    /// Classifier appended to the file name (e.g. `installer`).
    pub fn resource_suffix(mut self, suffix: &'a str) -> Self {
        self.resource_suffix = Some(suffix);
        self
    }

    /// File extension; defaults to `jar` when unset.
    pub fn file_ext(mut self, ext: &'a str) -> Self {
        self.file_ext = Some(ext);
        self
    }

    fn coords_to_relpath(&self) -> PathBuf {
        let parts: Vec<&str> = self.coords.splitn(3, ':').collect();
        let (group_id, artifact_id, version) = (parts[0], parts[1], parts[2]);
        let file_ext = self.file_ext.unwrap_or("jar");
        let jar_name = match self.resource_suffix {
            Some(suffix) => format!("{artifact_id}-{version}-{suffix}.{file_ext}"),
            None => format!("{artifact_id}-{version}.{file_ext}"),
        };
        let mut relative_path = PathBuf::from(group_id.replace('.', "/"));
        relative_path.extend([artifact_id, version, &jar_name]);
        relative_path
    }

    /// Download the artifact into `dest_path` (mirroring the repo's directory
    /// layout) and verify it against the sibling `.sha1`. Returns the path
    /// written.
    pub async fn download(self, client: &Client, dest_path: &Path) -> Result<PathBuf, Error> {
        let relative_path = self.coords_to_relpath();
        let url = format!("{}/{}", self.repo_url, relative_path.to_string_lossy());
        let file_path = dest_path.join(&relative_path);

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