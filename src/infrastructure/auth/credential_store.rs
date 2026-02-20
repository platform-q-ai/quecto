// Credential store: file-based storage for API tokens and OAuth credentials.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::error::DomainError;

/// How the credential was obtained.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AuthMethod {
    #[serde(rename = "token")]
    Token,
    #[serde(rename = "oauth")]
    OAuth,
}

impl AuthMethod {
    pub fn as_str(&self) -> &str {
        match self {
            AuthMethod::Token => "token",
            AuthMethod::OAuth => "oauth",
        }
    }
}

/// A stored credential for a provider.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Credential {
    pub provider: String,
    pub token: String,
    pub method: AuthMethod,
    /// Unix timestamp (seconds) when this credential expires, or None if no expiry.
    pub expires_at: Option<i64>,
}

impl Credential {
    /// Check if this credential is expired.
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = chrono::Utc::now().timestamp();
            now >= expires_at
        } else {
            false
        }
    }

    /// Return the status string: "active" or "expired".
    pub fn status(&self) -> &str {
        if self.is_expired() {
            "expired"
        } else {
            "active"
        }
    }
}

/// Serializable credentials file.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CredentialsFile {
    credentials: HashMap<String, Credential>,
}

/// File-based credential store. Stores credentials as JSON in a single file.
#[derive(Debug)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    /// Create a credential store at the given base directory.
    /// Credentials will be stored in `<base_dir>/credentials.json`.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            path: base_dir.as_ref().join("credentials.json"),
        }
    }

    /// Load all credentials from disk.
    fn load_all(&self) -> Result<HashMap<String, Credential>, DomainError> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let data = std::fs::read_to_string(&self.path)
            .map_err(|e| DomainError::Config(format!("failed to read credentials: {}", e)))?;
        let file: CredentialsFile = serde_json::from_str(&data)
            .map_err(|e| DomainError::Config(format!("failed to parse credentials: {}", e)))?;
        Ok(file.credentials)
    }

    /// Save all credentials to disk.
    fn save_all(&self, credentials: &HashMap<String, Credential>) -> Result<(), DomainError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DomainError::Config(format!("failed to create credentials dir: {}", e))
            })?;
        }
        let file = CredentialsFile {
            credentials: credentials.clone(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| DomainError::Config(format!("failed to serialize credentials: {}", e)))?;
        std::fs::write(&self.path, json)
            .map_err(|e| DomainError::Config(format!("failed to write credentials: {}", e)))?;
        Ok(())
    }

    /// Store a credential for a provider.
    pub fn store(&self, credential: Credential) -> Result<(), DomainError> {
        let mut all = self.load_all()?;
        all.insert(credential.provider.clone(), credential);
        self.save_all(&all)
    }

    /// Get a credential for a provider. Returns None if not found.
    pub fn get(&self, provider: &str) -> Result<Option<Credential>, DomainError> {
        let all = self.load_all()?;
        Ok(all.get(provider).cloned())
    }

    /// Check if a credential exists for a provider.
    pub fn exists(&self, provider: &str) -> Result<bool, DomainError> {
        let all = self.load_all()?;
        Ok(all.contains_key(provider))
    }

    /// Remove a credential for a specific provider.
    pub fn remove(&self, provider: &str) -> Result<(), DomainError> {
        let mut all = self.load_all()?;
        all.remove(provider);
        self.save_all(&all)
    }

    /// Remove all credentials.
    pub fn remove_all(&self) -> Result<(), DomainError> {
        self.save_all(&HashMap::new())
    }

    /// List all stored credentials.
    pub fn list(&self) -> Result<Vec<Credential>, DomainError> {
        let all = self.load_all()?;
        Ok(all.into_values().collect())
    }

    /// Get a summary of all credentials for the auth status display.
    pub fn status_summary(&self) -> Result<Vec<CredentialStatus>, DomainError> {
        let all = self.load_all()?;
        if all.is_empty() {
            return Ok(vec![]);
        }
        Ok(all
            .values()
            .map(|c| CredentialStatus {
                provider: c.provider.clone(),
                method: c.method.as_str().to_string(),
                status: c.status().to_string(),
            })
            .collect())
    }
}

/// Summary of a credential's status.
#[derive(Debug, Clone)]
pub struct CredentialStatus {
    pub provider: String,
    pub method: String,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_credential(provider: &str, token: &str, method: AuthMethod) -> Credential {
        Credential {
            provider: provider.to_string(),
            token: token.to_string(),
            method,
            expires_at: None,
        }
    }

    fn make_expired_credential(provider: &str) -> Credential {
        Credential {
            provider: provider.to_string(),
            token: "expired-token".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0), // epoch — always expired
        }
    }

    #[test]
    fn test_store_and_get() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        let cred = make_credential("openai", "sk-test", AuthMethod::Token);
        store.store(cred).unwrap();

        let loaded = store.get("openai").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.token, "sk-test");
        assert_eq!(loaded.method, AuthMethod::Token);
    }

    #[test]
    fn test_get_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        let loaded = store.get("openai").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_exists() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        assert!(!store.exists("openai").unwrap());
        store
            .store(make_credential("openai", "sk-test", AuthMethod::Token))
            .unwrap();
        assert!(store.exists("openai").unwrap());
    }

    #[test]
    fn test_remove() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        store
            .store(make_credential("openai", "sk-test", AuthMethod::Token))
            .unwrap();
        assert!(store.exists("openai").unwrap());

        store.remove("openai").unwrap();
        assert!(!store.exists("openai").unwrap());
    }

    #[test]
    fn test_remove_all() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        store
            .store(make_credential("openai", "sk-1", AuthMethod::Token))
            .unwrap();
        store
            .store(make_credential("anthropic", "sk-2", AuthMethod::Token))
            .unwrap();

        store.remove_all().unwrap();
        assert!(!store.exists("openai").unwrap());
        assert!(!store.exists("anthropic").unwrap());
    }

    #[test]
    fn test_credential_expired() {
        let cred = make_expired_credential("test");
        assert!(cred.is_expired());
        assert_eq!(cred.status(), "expired");
    }

    #[test]
    fn test_credential_not_expired() {
        let cred = Credential {
            provider: "test".to_string(),
            token: "token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX), // far future
        };
        assert!(!cred.is_expired());
        assert_eq!(cred.status(), "active");
    }

    #[test]
    fn test_credential_no_expiry() {
        let cred = make_credential("test", "token", AuthMethod::Token);
        assert!(!cred.is_expired());
        assert_eq!(cred.status(), "active");
    }

    #[test]
    fn test_status_summary_empty() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        let summary = store.status_summary().unwrap();
        assert!(summary.is_empty());
    }

    #[test]
    fn test_status_summary_with_credentials() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        store
            .store(make_credential("openai", "sk-1", AuthMethod::OAuth))
            .unwrap();
        store.store(make_expired_credential("anthropic")).unwrap();

        let summary = store.status_summary().unwrap();
        assert_eq!(summary.len(), 2);

        let openai = summary.iter().find(|s| s.provider == "openai").unwrap();
        assert_eq!(openai.status, "active");
        assert_eq!(openai.method, "oauth");

        let anthropic = summary.iter().find(|s| s.provider == "anthropic").unwrap();
        assert_eq!(anthropic.status, "expired");
    }

    #[test]
    fn test_list() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        store
            .store(make_credential("openai", "sk-1", AuthMethod::Token))
            .unwrap();
        store
            .store(make_credential("anthropic", "sk-2", AuthMethod::OAuth))
            .unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_overwrite_credential() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        store
            .store(make_credential("openai", "old-token", AuthMethod::Token))
            .unwrap();
        store
            .store(make_credential("openai", "new-token", AuthMethod::OAuth))
            .unwrap();

        let loaded = store.get("openai").unwrap().unwrap();
        assert_eq!(loaded.token, "new-token");
        assert_eq!(loaded.method, AuthMethod::OAuth);
    }
}
