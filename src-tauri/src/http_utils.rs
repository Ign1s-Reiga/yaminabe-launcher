use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::path::PathBuf;
use yaminabe_launcher_shared::error::Error;

/// Entry point for the JSON-fetching builder. Default method is GET; chain
/// `.method(...)` / `.payload(...)` / `.form(...)` / `.query(...)` / `.header(...)`
/// before `.send().await`. The client and url are borrowed for the duration of
/// the chain, so a request must be built and sent within one expression — the
/// usual fluent usage; the builder is never meant to outlive the call.
pub fn fetch_json<'a>(client: &'a Client, url: &'a str) -> FetchJson<'a> {
    FetchJson {
        client,
        url,
        method: Method::GET,
        body: None,
        query: Vec::new(),
        headers: Vec::new(),
    }
}

/// Fluent builder for a single JSON-decoded HTTP request. All inputs are
/// borrowed (`&'a`), so the request must be sent within the expression that
/// builds it. `.payload()` defers serialization until `.send()` so a Serialize
/// failure surfaces as the Result of `.send()` instead of breaking the chain.
pub struct FetchJson<'a> {
    client: &'a Client,
    url: &'a str,
    method: Method,
    body: Option<RequestBody<'a>>,
    query: Vec<(&'a str, &'a str)>,
    headers: Vec<(&'a str, &'a str)>,
}

/// At most one request body: a JSON payload or a urlencoded form.
enum RequestBody<'a> {
    Json(Result<Vec<u8>, Error>),
    Form(Vec<(&'a str, &'a str)>),
}

impl<'a> FetchJson<'a> {
    /// Override the HTTP method. Default is `Method::GET`.
    pub fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    /// Serialize `data` to JSON and attach it as the request body, replacing any
    /// previously-set body. A failing `Serialize` impl is reported by `.send()`
    /// rather than panicking here.
    pub fn payload<T: Serialize>(mut self, data: T) -> Self {
        self.body = Some(RequestBody::Json(serde_json::to_vec(&data).map_err(Error::ParseJson)));
        self
    }

    /// Attach an application/x-www-form-urlencoded body, replacing any
    /// previously-set body.
    pub fn form(mut self, params: &[(&'a str, &'a str)]) -> Self {
        self.body = Some(RequestBody::Form(params.to_vec()));
        self
    }

    /// Append query-string parameters. Call repeatedly to combine multiple
    /// sources; pairs are passed through to `RequestBuilder::query`.
    pub fn query(mut self, params: &[(&'a str, &'a str)]) -> Self {
        self.query.extend_from_slice(params);
        self
    }

    /// Add a single request header. Invalid names or values surface as an
    /// error on `.send()`.
    pub fn header(mut self, name: &'a str, value: &'a str) -> Self {
        self.headers.push((name, value));
        self
    }

    /// Build the request via `client.request(method, url)`, send it, and
    /// decode the response body as JSON. The fact that we construct the
    /// `RequestBuilder` here — not at chain time — is what lets `.method(...)`
    /// actually change the verb (reqwest's `RequestBuilder` doesn't let you
    /// mutate its method after creation).
    pub async fn send<T: DeserializeOwned>(self) -> Result<T, Error> {
        let mut req = self.client.request(self.method, self.url);

        if !self.query.is_empty() {
            req = req.query(&self.query);
        }

        for (name, value) in &self.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| Error::Invalid(format!("invalid header name '{name}': {e}")))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|e| Error::Invalid(format!("invalid header value for '{name}': {e}")))?;
            req = req.header(header_name, header_value);
        }

        match self.body {
            Some(RequestBody::Form(form)) => req = req.form(&form),
            Some(RequestBody::Json(bytes)) => {
                req = req.header(CONTENT_TYPE, "application/json").body(bytes?);
            }
            None => {}
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(Error::HttpRequestRejected(resp.status().as_u16(), self.url.to_string()));
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
    let expected_sha1 = if expected_sha1.is_empty() { None } else { Some(expected_sha1) };

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
pub fn verify_sha1(bytes: &[u8], expected_sha1: Option<&str>, resource: &str) -> Result<(), Error> {
    if let Some(sha1) = expected_sha1 {
        let trimmed = sha1.trim();
        let actual = sha1_hex(bytes);
        if actual != trimmed {
            return Err(Error::ChecksumMismatch {
                resource: resource.to_string(),
                sha1: trimmed.to_string(),
                hex: actual,
            });
        }
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
    let expected_sha1 = if expected_sha1.is_empty() { None } else { Some(expected_sha1) };

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), url.to_string()));
    }
    let bytes = resp.bytes().await.map_err(Error::InvalidResponse)?.to_vec();
    verify_sha1(&bytes, expected_sha1, resource)?;
    Ok(bytes)
}
