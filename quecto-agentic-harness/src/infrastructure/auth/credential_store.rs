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
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
    /// Compatibility location used for reads and OAuth migration.
    migration_path: Option<PathBuf>,
    /// Whether fallback credentials should be included in a mutation snapshot.
    migrate_on_write: bool,
    /// An explicitly configured file is a security boundary, not a hint. Keep
    /// malformed relative values fail-closed instead of silently using HOME.
    require_absolute_path: bool,
    /// Dedicated OAuth files have strict owner-only path and mode checks.
    dedicated: bool,
}

/// The result of trying to publish a refresh performed under a lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshLeaseOutcome {
    /// The refresh was based on the current generation and is now authoritative.
    Committed(Credential),
    /// Another writer committed a newer generation while this refresh ran.
    Superseded(Credential),
    /// The credential was revoked while this refresh was in flight.
    Revoked,
}

/// An exclusive cross-process refresh lease. The lock remains held for the
/// entire network request, so no second refresh can consume the same rotating
/// refresh token. Callers must either `commit` or drop the lease.
pub struct CredentialRefreshLease {
    store: CredentialStore,
    lock: Option<std::fs::File>,
    provider: String,
    generation: String,
    credential: Credential,
}

impl std::fmt::Debug for CredentialRefreshLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRefreshLease")
            .field("provider", &self.provider)
            .field("generation", &self.generation)
            .field("credential", &self.credential)
            .finish()
    }
}

impl CredentialStore {
    /// Create the general (legacy-compatible) credential store.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        let base = base_dir.as_ref();
        // Also read the dedicated store when no legacy file exists. This keeps
        // callers of the historical constructor source-compatible while OAuth
        // writes are provisioned in the narrow store.
        Self {
            path: base.join("credentials.json"),
            migration_path: Some(base.join("oauth-credentials.json")),
            migrate_on_write: false,
            require_absolute_path: false,
            dedicated: false,
        }
    }

    /// Create the narrow OAuth credential store. OAuth credentials are kept in
    /// their own file so container runtimes never need access to unrelated
    /// credentials. The old combined file is read once as a migration fallback.
    pub fn oauth(base_dir: impl AsRef<Path>) -> Self {
        let base = base_dir.as_ref();
        // A container may explicitly select a mounted *file* without exposing
        // the host's general Quecto home. The variable is intentionally a
        // narrow seam: it names only the OAuth JSON file, never a base dir.
        let explicit_path = std::env::var_os("QUECTO_OAUTH_CREDENTIALS_FILE");
        let path = explicit_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| base.join("oauth-credentials.json"));
        Self::oauth_path(path, base, explicit_path.is_some())
    }

    /// Construct the dedicated store from an explicit file path. This is
    /// useful for callers that already validated configuration and for tests;
    /// unlike `new`, it never accepts a directory or relative path.
    pub fn oauth_file(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        Self::oauth_path(path, Path::new("."), true)
    }

    fn oauth_path(path: PathBuf, base: &Path, explicit_path: bool) -> Self {
        Self {
            path,
            // An explicitly mounted file is a complete contract. Never look
            // through it at a host-side legacy file (which could expose an
            // unrelated credential set).
            migration_path: (!explicit_path).then(|| base.join("credentials.json")),
            migrate_on_write: true,
            require_absolute_path: explicit_path,
            dedicated: true,
        }
    }

    /// Load all credentials from disk as a snapshot.
    ///
    /// This is intentionally stateless: each call re-reads the file from disk.
    /// Correct for CLI (no stale state); for long-running processes, call once
    /// at startup and pass the snapshot to resolution functions.
    fn read_file(path: &Path) -> Result<HashMap<String, Credential>, DomainError> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| DomainError::Config(format!("failed to read credentials: {}", e)))?;
        let file: CredentialsFile = serde_json::from_str(&data)
            .map_err(|e| DomainError::Config(format!("failed to parse credentials: {}", e)))?;
        Ok(file.credentials)
    }

    fn validate_path(&self) -> Result<(), DomainError> {
        if self.require_absolute_path && !self.path.is_absolute() {
            return Err(DomainError::Config(
                "QUECTO_OAUTH_CREDENTIALS_FILE must be an absolute file path".to_string(),
            ));
        }
        if self.path.file_name().is_none() {
            return Err(DomainError::Config(
                "credential path must name a file".to_string(),
            ));
        }
        // The explicit FILE contract is a security boundary. Check every
        // existing parent component without following symlinks before any
        // read, lock, or write. A symlinked parent would otherwise redirect
        // the supposedly dedicated file to an arbitrary location.
        if self.require_absolute_path {
            let mut parent = if self.path.is_absolute() {
                PathBuf::from(std::path::MAIN_SEPARATOR.to_string())
            } else {
                PathBuf::new()
            };
            if let Some(relative_parent) = self.path.parent() {
                for component in relative_parent.components() {
                    if matches!(component, std::path::Component::RootDir) {
                        continue;
                    }
                    if !matches!(component, std::path::Component::Normal(_)) {
                        return Err(DomainError::Config(
                            "credential path contains an unsafe component".to_string(),
                        ));
                    }
                    parent.push(component.as_os_str());
                    if let Ok(metadata) = std::fs::symlink_metadata(&parent) {
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(DomainError::Config(
                                "credential path has an unsafe parent".to_string(),
                            ));
                        }
                    }
                }
            }
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&self.path) {
            if metadata.file_type().is_symlink() {
                return Err(DomainError::Config(
                    "refusing symlink credential file".to_string(),
                ));
            }
            if self.dedicated {
                if !metadata.is_file() {
                    return Err(DomainError::Config(
                        "credential path must name a regular file".to_string(),
                    ));
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::{MetadataExt, PermissionsExt};
                    if metadata.uid() != unsafe { libc::geteuid() } {
                        return Err(DomainError::Config(
                            "credential file is not owned by the current user".to_string(),
                        ));
                    }
                    if metadata.permissions().mode() & 0o077 != 0 {
                        return Err(DomainError::Config(
                            "credential file permissions must be owner-only".to_string(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn load_snapshot(&self) -> Result<HashMap<String, Credential>, DomainError> {
        self.validate_path()?;
        if self.path.exists() { return Self::read_file(&self.path); }
        if let Some(legacy) = &self.migration_path {
            if legacy.exists() {
                return Ok(Self::read_file(legacy)?.into_iter().filter(|(_, c)| c.method == AuthMethod::OAuth).collect());
            }
        }
        Ok(HashMap::new())
    }

    /// Snapshot used by mutations. It deliberately excludes compatibility
    /// fallback data: an API-token write must never copy OAuth secrets from the
    /// dedicated file into the broad legacy file.
    fn primary_snapshot(&self) -> Result<HashMap<String, Credential>, DomainError> {
        if self.path.exists() { Self::read_file(&self.path) } else { Ok(HashMap::new()) }
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
        self.validate_path()?;
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
        let mut all = if self.migrate_on_write {
            self.load_snapshot()?
        } else {
            self.primary_snapshot()?
        };
        all.insert(credential.provider.clone(), credential);
        self.save_all(&all)
    }

    /// Acquire an exclusive refresh lease for an OAuth credential. The lock is
    /// deliberately held until [`CredentialRefreshLease::commit`] or drop,
    /// including while the caller performs network I/O.
    pub fn begin_refresh(
        &self,
        provider: &str,
    ) -> Result<Option<CredentialRefreshLease>, DomainError> {
        let lock = self.lock_exclusive()?;
        let all = if self.migrate_on_write {
            self.load_snapshot()?
        } else {
            self.primary_snapshot()?
        };
        let Some(credential) = all.get(provider).cloned() else {
            drop(lock);
            return Ok(None);
        };
        if credential.method != AuthMethod::OAuth
            || credential
                .refresh_token
                .as_deref()
                .is_none_or(str::is_empty)
        {
            drop(lock);
            return Ok(None);
        }
        let generation = credential_generation(&credential);
        Ok(Some(CredentialRefreshLease {
            store: self.clone(),
            lock: Some(lock),
            provider: provider.to_string(),
            generation,
            credential,
        }))
    }

    /// Revoke a credential under the same lock used by refresh leases. A
    /// refresh already in flight cannot publish after revocation: either the
    /// lease still owns the lock (so revocation waits), or it observes the
    /// changed/missing generation and returns `Revoked`.
    pub fn revoke(&self, provider: &str) -> Result<bool, DomainError> {
        self.remove(provider)
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
        let mut all = if self.migrate_on_write {
            self.load_snapshot()?
        } else {
            self.primary_snapshot()?
        };
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
        let mut all = self.primary_snapshot()?;
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

impl CredentialRefreshLease {
    /// Credential snapshot and refresh token captured when this lease began.
    pub fn credential(&self) -> &Credential {
        &self.credential
    }

    /// Commit a refreshed credential if this lease still represents the same
    /// generation. The lock is released after the atomic replacement.
    pub fn commit(mut self, mut refreshed: Credential) -> Result<RefreshLeaseOutcome, DomainError> {
        if refreshed.provider != self.provider || refreshed.method != AuthMethod::OAuth {
            return Err(DomainError::Config(
                "refresh lease commit has incompatible credential".to_string(),
            ));
        }
        let all = if self.store.migrate_on_write {
            self.store.load_snapshot()?
        } else {
            self.store.primary_snapshot()?
        };
        let Some(existing) = all.get(&self.provider) else {
            self.lock.take();
            return Ok(RefreshLeaseOutcome::Revoked);
        };
        if credential_generation(existing) != self.generation {
            let current = existing.clone();
            self.lock.take();
            return Ok(RefreshLeaseOutcome::Superseded(current));
        }
        // Keep the provider identity from the lease, rather than allowing a
        // caller to accidentally publish into a different slot.
        refreshed.provider.clone_from(&self.provider);
        let mut updated = all;
        updated.insert(self.provider.clone(), refreshed.clone());
        self.store.save_all(&updated)?;
        self.lock.take();
        Ok(RefreshLeaseOutcome::Committed(refreshed))
    }
}

impl Drop for CredentialRefreshLease {
    fn drop(&mut self) {
        // Dropping the file releases flock(2), including on cancellation or
        // network failure. Explicit `take` in commit makes release obvious.
        let _ = self.lock.take();
    }
}

fn credential_generation(credential: &Credential) -> String {
    // Hash only non-secret metadata and a stable length-delimited token digest;
    // the generation is used for equality, never exposed as a credential.
    let mut value = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    credential.provider.hash(&mut value);
    credential.token.hash(&mut value);
    credential.refresh_token.hash(&mut value);
    credential.expires_at.hash(&mut value);
    format!("{:016x}", value.finish())
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
