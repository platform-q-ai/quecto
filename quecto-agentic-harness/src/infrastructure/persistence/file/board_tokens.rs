//! [`BoardTokens`] in a file (epic #2705 B5), until the SQLite store keeps
//! them as admin revisions: `tokens.json`, mode 0600, in the server's state
//! directory, mode 0700, both this user's. A token is 32 random bytes,
//! base64url; only its SHA-256 is written, every write is atomic, and a
//! presented token is compared with every digest in constant time. A file
//! that is shared, not this user's or not as written is refused, never
//! read as empty.
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::atomic_write::atomic_write;
use crate::application::project_board::ports::{
    BoardTokens, IssuedToken, MAX_TOKENS, TokenError, TokenRecord,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::shared::secret_compare::constant_time_eq;

/// The tokens file's name in the state directory.
pub const TOKENS_FILE: &str = "tokens.json";

/// The most bytes a tokens file may hold (256 tokens take about 40 KiB).
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// A digest: SHA-256.
type Digested = [u8; 32];

/// The file's shape, as written.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    tokens: Vec<StoredToken>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredToken {
    name: String,
    sha256: String,
    created: String,
}

/// A held token: its record and digest.
struct Held {
    record: TokenRecord,
    digest: Digested,
}

/// The device tokens in `<state>/tokens.json`.
#[derive(Debug)]
pub struct FileBoardTokens {
    path: PathBuf,
    /// Serialises this process's read-change-write of the file.
    writing: Mutex<()>,
}

impl FileBoardTokens {
    /// The tokens in `state`, made mode 0700 if missing; a file there must
    /// read.
    /// # Errors
    /// A relative, shared or foreign directory, or a file refused.
    pub fn open(state: &Path) -> Result<Self, String> {
        private_dir(state)?;
        let tokens = Self {
            path: state.join(TOKENS_FILE),
            writing: Mutex::new(()),
        };
        tokens.held().map_err(|error| error.to_string())?;
        Ok(tokens)
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The tokens the file holds: none while it is missing.
    fn held(&self) -> Result<Vec<Held>, TokenError> {
        let refused = |why: String| {
            TokenError::Unavailable(format!("the tokens file {}: {why}", self.path.display()))
        };
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(refused(error.to_string())),
        };
        match (
            meta.file_type().is_file(),
            meta.uid() == this_uid(),
            meta.permissions().mode() & 0o777 == 0o600,
            meta.len() <= MAX_FILE_BYTES,
        ) {
            (true, true, true, true) => {}
            (false, _, _, _) => return Err(refused("not a regular file".into())),
            (true, false, _, _) => return Err(refused("not owned by this user".into())),
            (true, true, false, _) => return Err(refused("must be mode 0600".into())),
            (true, true, true, false) => return Err(refused("past 1 MiB".into())),
        }
        let mut text = String::new();
        fs::File::open(&self.path)
            .and_then(|file| file.take(MAX_FILE_BYTES).read_to_string(&mut text))
            .map_err(|error| refused(error.to_string()))?;
        let stored: Stored =
            serde_json::from_str(&text).map_err(|error| refused(error.to_string()))?;
        stored
            .tokens
            .into_iter()
            .map(|token| held(token).ok_or_else(|| refused("an entry is not as written".into())))
            .collect()
    }

    /// Writes `held` as the file, atomically, mode 0600.
    fn store(&self, held: &[Held]) -> Result<(), TokenError> {
        let stored = Stored {
            tokens: held
                .iter()
                .map(|held| StoredToken {
                    name: held.record.name.as_str().to_string(),
                    sha256: hex(&held.digest),
                    created: held.record.created.as_str().to_string(),
                })
                .collect(),
        };
        let unavailable = |why: String| {
            TokenError::Unavailable(format!("the tokens file {}: {why}", self.path.display()))
        };
        let text = serde_json::to_vec_pretty(&stored).map_err(|e| unavailable(e.to_string()))?;
        atomic_write(&self.path, &text, Some(0o600)).map_err(|e| unavailable(e.to_string()))
    }

    /// Runs `change` on the held tokens under the write lock.
    fn changing<T>(
        &self,
        change: impl FnOnce(&mut Vec<Held>) -> Result<T, TokenError>,
    ) -> Result<T, TokenError> {
        // The lock guards no data, so a poisoned one is as good as any.
        let _writing = self
            .writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut held = self.held()?;
        let changed = change(&mut held)?;
        self.store(&held)?;
        Ok(changed)
    }
}

impl BoardTokens for FileBoardTokens {
    fn add(&self, name: &Slug, created: &Timestamp) -> Result<IssuedToken, TokenError> {
        // B5 red: a stub that keeps nothing.
        if self.path.exists() || !self.path.exists() {
            return Err(TokenError::Unavailable(format!(
                "{name:?} {created:?}: not yet"
            )));
        }
        self.changing(|held| {
            match (
                held.iter().any(|held| &held.record.name == name),
                held.len() < MAX_TOKENS,
            ) {
                (false, true) => {}
                (true, _) => return Err(TokenError::Exists(name.clone())),
                (false, false) => return Err(TokenError::Full),
            }
            let mut bytes = [0u8; 32];
            rand::rngs::OsRng
                .try_fill_bytes(&mut bytes)
                .map_err(|error| {
                    TokenError::Unavailable(format!("no randomness for a token: {error}"))
                })?;
            let secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            let record = TokenRecord {
                name: name.clone(),
                created: created.clone(),
            };
            held.push(Held {
                record: record.clone(),
                digest: Sha256::digest(secret.as_bytes()).into(),
            });
            Ok(IssuedToken { record, secret })
        })
    }

    fn revoke(&self, name: &Slug) -> Result<(), TokenError> {
        self.changing(|held| {
            let before = held.len();
            held.retain(|held| &held.record.name != name);
            match held.len() < before {
                true => Ok(()),
                false => Err(TokenError::NoSuchToken(name.clone())),
            }
        })
    }

    fn list(&self) -> Result<Vec<TokenRecord>, TokenError> {
        let mut records: Vec<TokenRecord> =
            self.held()?.into_iter().map(|held| held.record).collect();
        records.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(records)
    }

    fn verify(&self, presented: &str) -> Result<Option<Slug>, TokenError> {
        let presented = hex(&Sha256::digest(presented.as_bytes()).into());
        // Every digest is compared whole, in constant time and with no early
        // end, so the time taken says nothing of which (if any) matched.
        let mut found = None;
        for held in self.held()? {
            if constant_time_eq(&hex(&held.digest), &presented) {
                found = Some(held.record.name);
            }
        }
        Ok(found)
    }
}

/// A stored entry as a held token, when every field is as written.
fn held(token: StoredToken) -> Option<Held> {
    let name = Slug::parse(&token.name).ok()?;
    let created = Timestamp::parse(&token.created).ok()?;
    let digest = unhex(&token.sha256)?;
    Some(Held {
        record: TokenRecord { name, created },
        digest,
    })
}

fn hex(digest: &Digested) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 64 lowercase hex digits as a digest.
fn unhex(text: &str) -> Option<Digested> {
    let bytes = text.as_bytes();
    let lower = |byte: &u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte);
    match bytes.len() == 64 && bytes.iter().all(lower) {
        true => {}
        false => return None,
    }
    let mut digest = [0u8; 32];
    for (slot, pair) in digest.iter_mut().zip(bytes.chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(digest)
}

/// `dir` as an absolute directory of mode 0700 of this user's, made so
/// (with its parents) if missing.
fn private_dir(dir: &Path) -> Result<(), String> {
    let refused = |why: &str| format!("the state directory {}: {why}", dir.display());
    match dir.is_absolute() {
        true => {}
        false => return Err(refused("not absolute")),
    }
    match fs::symlink_metadata(dir) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|error| refused(&error.to_string()))?;
        }
        Err(error) => return Err(refused(&error.to_string())),
    }
    let meta = fs::symlink_metadata(dir).map_err(|error| refused(&error.to_string()))?;
    match (
        meta.is_dir(),
        meta.uid() == this_uid(),
        meta.permissions().mode() & 0o777 == 0o700,
    ) {
        (true, true, true) => Ok(()),
        (false, _, _) => Err(refused("not a directory (a symlink is not followed)")),
        (true, false, _) => Err(refused("not owned by this user")),
        (true, true, false) => Err(refused("must be mode 0700 (owner only)")),
    }
}

fn this_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
#[path = "board_tokens_tests.rs"]
mod tests;
