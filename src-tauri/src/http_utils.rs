use reqwest::Client;
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use std::path::PathBuf;
use yaminabe_launcher_shared::error::Error;

pub async fn fetch_json<T>(
    client: &Client,
    url: &str,
    query: &[(&str, &str)],
    curseforge_api_key: Option<String>,
) -> Result<T, Error>
where
    T: DeserializeOwned
{
    let mut req = client.get(url);
    if let Some(key) = curseforge_api_key {
        req = req.header("x-api-key", key);
    }
    if !query.is_empty() {
        req = req.query(query);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), url.to_string()));
    }
    Ok(resp.json::<T>().await.map_err(Error::InvalidResponse)?)
}

pub async fn download_resource(
    client: &Client,
    url: &str,
    dest_path: PathBuf,
) -> Result<(), Error> {
    let req = client.get(url);
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), url.to_string()));
    }
    let bytes = resp.bytes().await
        .map_err(Error::InvalidResponse)?;
    std::fs::write(dest_path, &bytes)?;
    Ok(())
}

pub fn get_resource_name(url: &str) -> Option<&str> {
    if url.ends_with("/") {
        None
    } else {
        url.split("/").last()
    }
}

pub async fn download_from_maven(
    client: &Client,
    repo_url: &str,
    dep: String,
    jar_name_suffix: Option<&str>,
    file_ext: &str,
    dest_path: PathBuf,
) -> Result<(), Error> {
    let parts: Vec<&str> = dep.splitn(3, ':').collect();
    let (group_id, artifact_id, version) = (parts[0], parts[1], parts[2]);
    let group_url_path = group_id.replace('.', "/");
    let group_dir = group_id.replace('.', std::path::MAIN_SEPARATOR_STR);
    let jar_name = match jar_name_suffix {
        Some(suffix) => format!("{artifact_id}-{version}-{suffix}.{file_ext}"),
        None => format!("{artifact_id}-{version}.{file_ext}"),
    };
    let url = format!("{repo_url}/{group_url_path}/{artifact_id}/{version}/{jar_name}");
    let file_path = dest_path.join(&group_dir).join(artifact_id).join(version).join(&jar_name);

    let sha1 = client.get(format!("{url}.sha1")).send().await?
        .text().await.map_err(Error::InvalidResponse)?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), url.to_string()));
    }
    let bytes = resp.bytes().await.map_err(Error::InvalidResponse)?;

    let hex = Sha1::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if hex != sha1.trim() {
        return Err(Error::ChecksumMismatch { resource: dep, sha1, hex });
    }

    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&file_path, &bytes)?;
    Ok(())
}
