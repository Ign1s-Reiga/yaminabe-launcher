use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::path::PathBuf;
use yaminabe_launcher_shared::error::Error;

/// Entry point for the JSON-fetching builder. Default method is GET; chain
/// `.method(...)` / `.payload(...)` / `.query(...)` / `.header(...)` before
/// `.send().await` to customise. The shared `reqwest::Client` is positional
/// because every call site already has one (`state.http_client`); making it
/// the first argument keeps the "I have a client, fetch this URL" mental
/// model honest and avoids a runtime check for an unset client in `send()`.
pub fn fetch_json(client: &Client, url: &str) -> FetchJsonBuilder {
    FetchJsonBuilder {
        client: client.clone(),
        url: url.to_string(),
        method: Method::GET,
        payload: None,
        form: None,
        query: Vec::new(),
        headers: Vec::new(),
    }
}

/// Fluent builder for a single JSON-decoded HTTP request. Methods like
/// `.payload()` defer serialization until `.send()` so a Serialize failure
/// surfaces as the Result of `.send()` instead of breaking the chain.
pub struct FetchJsonBuilder {
    client: Client,
    url: String,
    method: Method,
    payload: Option<Result<Vec<u8>, Error>>,
    form: Option<Vec<(String, String)>>,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
}

impl FetchJsonBuilder {
    /// Override the HTTP method. Default is `Method::GET`.
    pub fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    /// Serialize `data` to JSON and attach it as the request body. If the
    /// type's `Serialize` impl fails, the error is propagated when `.send()`
    /// is called rather than panicking here. Clears any previously-set form.
    pub fn payload<T: Serialize>(mut self, data: T) -> Self {
        self.payload = Some(serde_json::to_vec(&data).map_err(Error::ParseJson));
        self.form = None;
        self
    }

    /// Attach an application/x-www-form-urlencoded body built from the given
    /// key/value pairs. Clears any previously-set JSON payload.
    pub fn form(mut self, params: &[(&str, &str)]) -> Self {
        let owned = params
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        self.form = Some(owned);
        self.payload = None;
        self
    }

    /// Append query-string parameters. Call repeatedly to combine multiple
    /// sources; pairs are passed through to `RequestBuilder::query`.
    pub fn query(mut self, params: &[(&str, &str)]) -> Self {
        for (k, v) in params {
            self.query.push(((*k).to_string(), (*v).to_string()));
        }
        self
    }

    /// Add a single request header. Invalid names or values surface as an
    /// error on `.send()`.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Build the request via `client.request(method, url)`, send it, and
    /// decode the response body as JSON. The fact that we construct the
    /// `RequestBuilder` here — not at chain time — is what lets `.method(...)`
    /// actually change the verb (reqwest's `RequestBuilder` doesn't let you
    /// mutate its method after creation).
    pub async fn send<T: DeserializeOwned>(self) -> Result<T, Error> {
        let mut req = self.client.request(self.method, &self.url);

        if !self.query.is_empty() {
            // Borrow as &[(&str, &str)] for the reqwest API.
            let q: Vec<(&str, &str)> = self.query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            req = req.query(&q);
        }

        for (name, value) in &self.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| Error::Invalid(format!("invalid header name '{name}': {e}")))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|e| Error::Invalid(format!("invalid header value for '{name}': {e}")))?;
            req = req.header(header_name, header_value);
        }

        if let Some(form) = self.form {
            req = req.form(&form);
        } else if let Some(payload_result) = self.payload {
            let bytes = payload_result?;
            req = req.header(CONTENT_TYPE, "application/json").body(bytes);
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(Error::HttpRequestRejected(
                resp.status().as_u16(),
                self.url,
            ));
        }
        resp.json::<T>().await.map_err(Error::InvalidResponse)
    }
}

pub async fn download_resource(
    client: &Client,
    url: &str,
    expected_sha1: &str,
    dest_path: PathBuf,
) -> Result<(), Error> {
    let resource = get_resource_name(url).unwrap_or_default();

    // An existing file that already matches needs no network round-trip; one
    // that does not match is stale/corrupt and must be replaced, so fall
    // through and overwrite it with verified bytes below.
    if dest_path.exists() {
        if let Ok(existing) = std::fs::read(&dest_path) {
            if verify_sha1(&existing, expected_sha1, resource).is_ok() {
                return Ok(());
            }
        }
    }

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), url.to_string()));
    }
    let bytes = resp.bytes().await
        .map_err(Error::InvalidResponse)?;
    verify_sha1(&bytes, expected_sha1, resource)?;
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write to a sibling temp file then rename into place. Rename is atomic, so
    // a concurrent reader (e.g. a mod loader scanning mods/ at launch) sees
    // either the old file or the complete new one, never a partial write.
    let file_name = dest_path.file_name()
        .ok_or_else(|| Error::Invalid(format!("download destination has no file name: {}", dest_path.display())))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".part");
    let tmp_path = dest_path.with_file_name(tmp_name);

    std::fs::write(&tmp_path, &bytes)?;
    if let Err(e) = std::fs::rename(&tmp_path, &dest_path) {
        std::fs::remove_file(&tmp_path).ok();
        return Err(e.into());
    }
    Ok(())
}

pub fn get_resource_name(url: &str) -> Option<&str> {
    if url.ends_with("/") {
        None
    } else {
        url.split("/").last()
    }
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Compare `bytes`'s SHA-1 to `expected_sha1` (lowercase hex, leading/trailing
/// whitespace tolerated) and return `Error::ChecksumMismatch` on a difference.
/// `resource` is the human-readable name surfaced in the error.
pub fn verify_sha1(bytes: &[u8], expected_sha1: &str, resource: &str) -> Result<(), Error> {
    let expected = expected_sha1.trim();
    let actual = sha1_hex(bytes);
    if actual != expected {
        return Err(Error::ChecksumMismatch {
            resource: resource.to_string(),
            sha1: expected.to_string(),
            hex: actual,
        });
    }
    Ok(())
}

/// GET `url`, verify the response body's SHA-1 matches `expected_sha1`, and
/// return the body bytes. The non-success-status branch becomes
/// `HttpRequestRejected`; the checksum branch becomes `ChecksumMismatch`.
pub async fn fetch_and_verify(
    client: &Client,
    url: &str,
    expected_sha1: &str,
    resource: &str,
) -> Result<Vec<u8>, Error> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), url.to_string()));
    }
    let bytes = resp.bytes().await.map_err(Error::InvalidResponse)?.to_vec();
    verify_sha1(&bytes, expected_sha1, resource)?;
    Ok(bytes)
}

pub async fn download_from_maven(
    client: &Client,
    repo_url: &str,
    coords: String,
    jar_name_suffix: Option<&str>,
    file_ext: &str,
    dest_path: PathBuf,
) -> Result<PathBuf, Error> {
    let parts: Vec<&str> = coords.splitn(3, ':').collect();
    let (group_id, artifact_id, version) = (parts[0], parts[1], parts[2]);
    let jar_name = match jar_name_suffix {
        Some(suffix) => format!("{artifact_id}-{version}-{suffix}.{file_ext}"),
        None => format!("{artifact_id}-{version}.{file_ext}"),
    };
    let mut relative_path = PathBuf::from(group_id.replace('.', "/"));
    relative_path.extend([artifact_id, version, &jar_name]);
    
    let url = format!("{repo_url}/{}", relative_path.to_string_lossy());
    let file_path = dest_path.join(&relative_path);

    let sha1 = client.get(format!("{url}.sha1")).send().await?
        .text().await.map_err(Error::InvalidResponse)?;

    let bytes = fetch_and_verify(client, &url, sha1.trim(), &coords).await?;

    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&file_path, &bytes)?;
    Ok(file_path)
}
