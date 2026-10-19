//! The external `auth.json` (#1840): OAuth credentials a compatible tool
//! stored under the user's data directory, read as the application's
//! `ExternalCredentialSource`. The file's shape stays here: an
//! `{"<provider>": {"type": "oauth", "access", "refresh", "expires"}}` map
//! whose `expires` is in milliseconds.

use std::path::{Path, PathBuf};

use crate::application::auth::ports::{
    ExternalCredential, ExternalCredentialSource, ExternalCredentialsError,
};
use crate::domain::auth::provider::LoginProvider;
use crate::infrastructure::auth::oauth::extract_openai_account_id;

/// The providers the file holds credentials for, in import order.
const IMPORTED: [LoginProvider; 2] = [LoginProvider::Anthropic, LoginProvider::OpenAi];

#[derive(Debug, Clone)]
pub struct AuthJsonFile {
    path: PathBuf,
}

impl AuthJsonFile {
    /// The file at `path`.
    pub fn at(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// The file where compatible tools keep it: `<data dir>/quecto/auth.json`.
    pub fn default_location() -> Self {
        Self::at(
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                .join("quecto")
                .join("auth.json"),
        )
    }
}

impl ExternalCredentialSource for AuthJsonFile {
    fn load(&self) -> Result<Vec<ExternalCredential>, ExternalCredentialsError> {
        let path = self.path.display().to_string();
        let exists = self.path.exists();
        if !exists {
            return Err(ExternalCredentialsError::NotFound { path });
        }
        let data = std::fs::read_to_string(&self.path).map_err(|error| {
            ExternalCredentialsError::Unreadable {
                path,
                reason: error.to_string(),
            }
        })?;
        let file: serde_json::Value = serde_json::from_str(&data)
            .map_err(|error| ExternalCredentialsError::Malformed(error.to_string()))?;
        Ok(IMPORTED
            .into_iter()
            .filter_map(|provider| oauth_entry(&file, provider))
            .collect())
    }
}

/// `provider`'s entry in `file` when it is an OAuth one.
fn oauth_entry(file: &serde_json::Value, provider: LoginProvider) -> Option<ExternalCredential> {
    let entry = file
        .get(provider.id())
        .filter(|entry| entry.get("type").and_then(|kind| kind.as_str()) == Some("oauth"))?;
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string()
    };
    let access_token = text("access");
    let account_id = match provider {
        LoginProvider::OpenAi => extract_openai_account_id(&access_token),
        LoginProvider::Anthropic | LoginProvider::Xai => None,
    };
    Some(ExternalCredential {
        provider,
        refresh_token: text("refresh"),
        expires_at: entry
            .get("expires")
            .and_then(|value| value.as_i64())
            .unwrap_or(0)
            / 1000,
        account_id,
        access_token,
    })
}
