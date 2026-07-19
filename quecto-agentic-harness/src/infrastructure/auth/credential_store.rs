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
///
/// `Debug` is manually implemented to redact the token field, preventing
/// accidental exposure of secrets in debug logs, panic backtraces, or
/// `unwrap()` failure messages.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Credential {
    pub provider: String,
    pub token: String,
    pub method: AuthMethod,
    /// Unix timestamp (seconds) when this credential expires, or None if no expiry.
    pub expires_at: Option<i64>,
    /// Refresh token for OAuth credentials. Used to obtain new access tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Account ID for OpenAI OAuth (chatgpt_account_id from JWT).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("provider", &self.provider)
            .field("token", &"[REDACTED]")
            .field("method", &self.method)
            .field("expires_at", &self.expires_at)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("account_id", &self.account_id)
            .finish()
    }
}

impl Credential {
    /// Check if this credential is expired.
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = crate::infrastructure::time::unix_timestamp_secs();
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

    /// Load all credentials from disk as a snapshot.
    ///
    /// This is intentionally stateless: each call re-reads the file from disk.
    /// Correct for CLI (no stale state); for long-running processes, call once
    /// at startup and pass the snapshot to resolution functions.
    pub fn load_snapshot(&self) -> Result<HashMap<String, Credential>, DomainError> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let data = std::fs::read_to_string(&self.path)
            .map_err(|e| DomainError::Config(format!("failed to read credentials: {}", e)))?;
        let file: CredentialsFile = serde_json::from_str(&data)
            .map_err(|e| DomainError::Config(format!("failed to parse credentials: {}", e)))?;
        Ok(file.credentials)
    }

    /// Get the path to the credentials file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Save all credentials to disk with restricted file permissions (0600).
    ///
    /// On Unix, writes the same-directory replacement file with mode 0o600 before
    /// atomically renaming it into place, avoiding write-then-chmod exposure.
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

        // Write a same-directory temporary file, then rename it over the target.
        // The previous credentials file remains intact until the replacement is
        // fully written, so a crash/kill during save does not leave an empty or
        // partially written credential store.
        #[cfg(unix)]
        let mode = Some(0o600);
        #[cfg(not(unix))]
        let mode = None;
        crate::infrastructure::atomic_write::atomic_write(&self.path, json.as_bytes(), mode)
            .map_err(|e| DomainError::Config(format!("failed to write credentials: {}", e)))?;

        Ok(())
    }

    /// Store a credential for a provider.
    pub fn store(&self, credential: Credential) -> Result<(), DomainError> {
        let mut all = self.load_snapshot()?;
        all.insert(credential.provider.clone(), credential);
        self.save_all(&all)
    }

    /// Get a credential for a provider. Returns None if not found.
    pub fn get(&self, provider: &str) -> Result<Option<Credential>, DomainError> {
        let all = self.load_snapshot()?;
        Ok(all.get(provider).cloned())
    }

    /// Check if a credential exists for a provider.
    pub fn exists(&self, provider: &str) -> Result<bool, DomainError> {
        let all = self.load_snapshot()?;
        Ok(all.contains_key(provider))
    }

    /// Remove a credential for a specific provider.
    /// Returns `true` if a credential was actually removed, `false` if none existed.
    pub fn remove(&self, provider: &str) -> Result<bool, DomainError> {
        let mut all = self.load_snapshot()?;
        let removed = all.remove(provider).is_some();
        self.save_all(&all)?;
        Ok(removed)
    }

    /// Remove all credentials.
    pub fn remove_all(&self) -> Result<(), DomainError> {
        self.save_all(&HashMap::new())
    }

    /// List all stored credentials.
    pub fn list(&self) -> Result<Vec<Credential>, DomainError> {
        let all = self.load_snapshot()?;
        Ok(all.into_values().collect())
    }

    /// Get a summary of all credentials for the auth status display.
    pub fn status_summary(&self) -> Result<Vec<CredentialStatus>, DomainError> {
        let all = self.load_snapshot()?;
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
#[path = "credential_store_cov_tests.rs"]
mod cov_tests;

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
            refresh_token: None,
            account_id: None,
        }
    }

    fn make_expired_credential(provider: &str) -> Credential {
        Credential {
            provider: provider.to_string(),
            token: "expired-token".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0), // epoch — always expired
            refresh_token: None,
            account_id: None,
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

        let removed = store.remove("openai").unwrap();
        assert!(removed);
        assert!(!store.exists("openai").unwrap());

        // Removing again should return false
        let removed_again = store.remove("openai").unwrap();
        assert!(!removed_again);
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
            refresh_token: None,
            account_id: None,
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

    #[test]
    fn test_path_returns_credentials_file_path() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());
        assert_eq!(store.path(), tmp.path().join("credentials.json"));
    }

    // --- Sandbox hardening: credential file permission tests ---

    /// Exercises the real `store()` → `save_all()` → `atomic_write()` path (not
    /// a hand-rolled simulation): a second `store()` call must fully replace
    /// the credentials file's content via the same-directory-temp-file +
    /// rename it performs internally, and must not leave the temp file behind.
    #[test]
    fn test_atomic_replacement_preserves_existing_credentials_until_rename() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());
        store
            .store(make_credential("openai", "old-token", AuthMethod::Token))
            .unwrap();
        let before = std::fs::read(store.path()).unwrap();

        // A concurrent atomic_write from an unrelated writer targeting the same
        // temp-name pattern must not corrupt the store's own credentials file:
        // it only ever becomes visible via `std::fs::rename`, which this store
        // never observes unless it performs the rename itself.
        let stray_tmp = tmp.path().join(".credentials.json.stray-writer.tmp");
        std::fs::write(&stray_tmp, b"not a credentials file").unwrap();
        assert_eq!(
            std::fs::read(store.path()).unwrap(),
            before,
            "an unrelated temp file beside the credential file must not alter current credentials"
        );
        std::fs::remove_file(&stray_tmp).unwrap();

        // The real replacement path: store() -> save_all() -> atomic_write().
        store
            .store(make_credential("openai", "new-token", AuthMethod::OAuth))
            .unwrap();

        assert_eq!(
            store.get("openai").unwrap().unwrap().token,
            "new-token",
            "the second store() call must be visible after atomic_write's rename"
        );
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from("credentials.json")],
            "atomic_write must not leave its temp file behind after a successful rename"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_credentials_file_created_with_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        store
            .store(make_credential("openai", "sk-test", AuthMethod::Token))
            .unwrap();

        let metadata = std::fs::metadata(store.path()).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected permissions 0600, got {:04o}", mode);
    }

    #[cfg(unix)]
    #[test]
    fn test_credentials_permissions_enforced_on_every_write() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());

        // Store a credential to create the file
        store
            .store(make_credential("openai", "sk-test", AuthMethod::Token))
            .unwrap();

        // Manually weaken the permissions
        let permissions = std::fs::Permissions::from_mode(0o644);
        std::fs::set_permissions(store.path(), permissions).unwrap();

        // Verify they were weakened
        let metadata = std::fs::metadata(store.path()).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o644);

        // Store another credential — permissions should be re-enforced
        store
            .store(make_credential("anthropic", "sk-new", AuthMethod::Token))
            .unwrap();

        let metadata = std::fs::metadata(store.path()).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "expected permissions 0600 after re-write, got {:04o}",
            mode
        );
    }
}
