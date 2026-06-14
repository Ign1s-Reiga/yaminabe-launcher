use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use yaminabe_launcher_shared::datamodels::AccountSummary;
use yaminabe_launcher_shared::error::Error;
use crate::accounts_path;

/// In-memory composite an account is hydrated to whenever the auth code needs
/// to actually mint or use tokens. Secrets live in the OS keyring; the
/// non-secret fields live in `accounts.json` via `MinecraftAccountRecord`.
#[derive(Debug, Clone)]
pub struct MinecraftAccount {
    pub uuid: String,
    pub username: String,
    pub mc_access_token: String,
    /// Epoch seconds (UTC) at which `mc_access_token` ceases to be valid.
    /// The MS refresh token is rotated by Microsoft on each refresh and has
    /// its own (~90-day) lifetime tracked server-side, so we don't store an
    /// expiry for it here.
    pub expires_at: i64,
    pub ms_refresh_token: String,
    pub xuid: String,
}

impl MinecraftAccount {
    /// `${auth_uuid}` needs a dashed UUID; the profile endpoint returns it raw.
    pub(crate) fn uuid_dashed(&self) -> String {
        format_uuid_dashed(&self.uuid)
    }

    pub(crate) fn summary(&self) -> AccountSummary {
        AccountSummary {
            uuid: self.uuid_dashed(),
            username: self.username.clone(),
        }
    }
}

/// Non-secret portion of an account; this is what gets serialized to
/// `accounts.json`. The two token fields move into the OS keyring under
/// `service=KEYRING_SERVICE, user=<uuid>` as a small JSON payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftAccountRecord {
    pub uuid: String,
    pub username: String,
    #[serde(alias = "mcExpiresAt")]
    pub expires_at: i64,
    #[serde(default)]
    pub xuid: String,
}

impl MinecraftAccountRecord {
    pub(crate) fn from_account(account: &MinecraftAccount) -> Self {
        Self {
            uuid: account.uuid.clone(),
            username: account.username.clone(),
            expires_at: account.expires_at,
            xuid: account.xuid.clone(),
        }
    }

    pub(crate) fn summary(&self) -> AccountSummary {
        AccountSummary {
            uuid: format_uuid_dashed(&self.uuid),
            username: self.username.clone(),
        }
    }
}

/// Secret payload written into the OS keyring as a single JSON blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MinecraftAccountSecret {
    pub mc_access_token: String,
    pub ms_refresh_token: String,
}

/// File-backed container for the records list plus the currently selected
/// account. Wrapped in a `Mutex` on `AppState`; persisted as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStore {
    #[serde(default)]
    pub accounts: Vec<MinecraftAccountRecord>,
    /// Stored as the dash-stripped UUID (same format as Mojang returns).
    #[serde(default)]
    pub selected: Option<String>,
}

impl AccountStore {
    pub(crate) fn save(&self) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(accounts_path(), json)?;
        Ok(())
    }

    pub(crate) fn upsert(&mut self, record: MinecraftAccountRecord) {
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.uuid == record.uuid) {
            *existing = record;
        } else {
            // A new account becomes selected if nothing else is — first login
            // immediately becomes the launch identity.
            if self.selected.is_none() {
                self.selected = Some(record.uuid.clone());
            }
            self.accounts.push(record);
        }
    }
}

pub(crate) fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn format_uuid_dashed(undashed: &str) -> String {
    if undashed.len() != 32 || undashed.contains('-') {
        return undashed.to_string();
    }
    format!(
        "{}-{}-{}-{}-{}",
        &undashed[0..8],
        &undashed[8..12],
        &undashed[12..16],
        &undashed[16..20],
        &undashed[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::format_uuid_dashed;

    #[test]
    fn dashes_undashed_uuid() {
        assert_eq!(
            format_uuid_dashed("12345678abcd1234abcd1234567890ab"),
            "12345678-abcd-1234-abcd-1234567890ab"
        );
    }

    #[test]
    fn passes_through_already_dashed() {
        let already = "12345678-abcd-1234-abcd-1234567890ab";
        assert_eq!(format_uuid_dashed(already), already);
    }
}