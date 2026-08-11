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

    /// Path of the cross-process lock file guarding load-mutate-store cycles
    /// (#1460). Lives alongside `credentials.json` as `credentials.json.lock`.
    pub fn lock_path(&self) -> std::path::PathBuf {
        let mut os = self.path.as_os_str().to_os_string();
        os.push(".lock");
        std::path::PathBuf::from(os)
    }

    /// Take the cross-process exclusive lock guarding load-mutate-store
    /// cycles (#1460). Blocks until any other process's lock is released;
    /// the lock is released when the returned handle is dropped.
    //
    // `File::lock` stabilized in 1.89; the crate's tests already call it, so
    // 1.89 is the real toolchain floor — clippy.toml's declared 1.85 predates
    // the #1460 locking work and awaits a coordinated MSRV bump.
    #[expect(clippy::incompatible_msrv)]
    fn lock_exclusive(&self) -> Result<std::fs::File, DomainError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DomainError::Config(format!("failed to create credentials dir: {}", e))
            })?;
        }
        // Mode 0600 like credentials.json itself: a world-readable lock file
        // would let any co-resident user take the exclusive lock and wedge
        // every credential write indefinitely.
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .mode(0o600)
                .open(self.lock_path())
        };
        #[cfg(not(unix))]
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_path());
        let file = file.map_err(|e| {
            DomainError::Config(format!("failed to open credentials lock file: {}", e))
        })?;
        file.lock()
            .map_err(|e| DomainError::Config(format!("failed to lock credentials file: {}", e)))?;
        Ok(file)
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
    ///
    /// The whole load-mutate-store cycle runs under the cross-process
    /// credentials lock (#1460): N agent processes refreshing tokens
    /// concurrently serialize here instead of losing each other's writes.
    pub fn store(&self, credential: Credential) -> Result<(), DomainError> {
        let _lock = self.lock_exclusive()?;
        let mut all = self.load_snapshot()?;
        all.insert(credential.provider.clone(), credential);
        self.save_all(&all)
    }

    /// Persist a refreshed OAuth credential unless another process already
    /// rotated it (#1460 review). `refreshed_from` is the refresh token this
    /// refresh consumed; if the on-disk credential's refresh token no longer
    /// matches it and that credential is still valid, another agent refreshed
    /// concurrently and its (newer) token family must not be overwritten —
    /// last-writer-wins here would persist a competing/stale token and, with
    /// strict-rotation providers, strand every agent on a revoked family.
    ///
    /// Returns the credential that is authoritative after the call: the one
    /// written, or the fresher on-disk one that was kept.
    pub fn store_refreshed(
        &self,
        credential: Credential,
        refreshed_from: &str,
    ) -> Result<Credential, DomainError> {
        let _lock = self.lock_exclusive()?;
        let mut all = self.load_snapshot()?;
        if let Some(existing) = all.get(&credential.provider)
            && existing.refresh_token.as_deref() != Some(refreshed_from)
            && !existing.is_expired()
        {
            return Ok(existing.clone());
        }
        all.insert(credential.provider.clone(), credential.clone());
        self.save_all(&all)?;
        Ok(credential)
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
        let _lock = self.lock_exclusive()?;
        let mut all = self.load_snapshot()?;
        let removed = all.remove(provider).is_some();
        self.save_all(&all)?;
        Ok(removed)
    }

    /// Remove all credentials.
    pub fn remove_all(&self) -> Result<(), DomainError> {
        let _lock = self.lock_exclusive()?;
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
#[path = "credential_store_tests.rs"]
mod tests;
