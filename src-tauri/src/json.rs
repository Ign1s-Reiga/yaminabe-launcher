use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use yaminabe_launcher_shared::error::Error;

pub fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T, Error> {
    let content = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

pub fn read_json_or_default<T>(path: impl AsRef<Path>) -> Result<T, Error>
where
    T: DeserializeOwned + Default,
{
    let path = path.as_ref();
    if !path.exists() {
        return Ok(T::default());
    }
    read_json(path)
}

pub fn write_json<T: Serialize>(path: impl AsRef<Path>, value: &T) -> Result<(), Error> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}
