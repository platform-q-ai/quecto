//! [`BoardTokens`] in `tokens.json` (epic #2705 B5; SQLite later), mode
//! 0600, SHA-256 digests only, in the locked state directory (0700). Read
//! once at open, verified from memory, each change written through
//! relative to the directory; a file not as written is refused.
use std::ffi::{CStr, CString};
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::application::project_board::ports::replication::{
    AdminRows, AdminToken, FollowTokens, FollowedTokens,
};
use crate::application::project_board::ports::{
    BoardTokens, IssuedToken, MAX_TOKENS, TokenError, TokenKind, TokenRecord,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::shared::secret_compare::constant_time_eq;
use crate::infrastructure::persistence::file::state_dir::{
    SERVER_HOLDS, StateDirLock, locked, private_dir, this_uid,
};

/// The tokens file's name in the state directory.
pub const TOKENS_FILE: &str = "tokens.json";
const TOKENS_NAME: &CStr = c"tokens.json";

/// A follower's own tokens, kept as they are set aside (B11): never over
/// an earlier set, which is kept as `-1` .. `-99` after it.
pub const PRE_FOLLOW_FILE: &str = "tokens.json.pre-follow";

/// The digest of the token set a follower last took from its primary (or
/// set aside its own for), beside the tokens (B11).
pub const FOLLOWED_FILE: &str = "tokens.json.followed";
const FOLLOWED_NAME: &CStr = c"tokens.json.followed";

/// A file's (inode, mtime, mtime ns, size), to tell a hand edit.
type Stamp = (u64, i64, i64, u64);

fn stamp(meta: &fs::Metadata) -> Stamp {
    (meta.ino(), meta.mtime(), meta.mtime_nsec(), meta.len())
}

const CHANGED_BEHIND: &str =
    "changed behind this server: restart it to pick up hand edits (edit only while it is stopped)";

/// The most bytes a tokens file may hold (256 tokens take about 40 KiB).
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// One entry of the file (a JSON array of them).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredToken {
    name: String,
    sha256: String,
    created: String,
    /// A replica token's (B11); absent for a device's, as in files before it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    replica: bool,
}

/// A held token: its record and SHA-256, as 64 lowercase hex digits.
#[derive(Debug, Clone)]
struct Held {
    record: TokenRecord,
    digest: String,
}

/// The device tokens in `<state>/tokens.json`.
#[derive(Debug)]
pub struct FileBoardTokens {
    /// The tokens as written: a snapshot `verify` takes without waiting.
    held: RwLock<Arc<Vec<Held>>>,
    /// Serialises changes, holding the file's stamp as last read or written.
    writing: Mutex<Option<Stamp>>,
    /// The state directory, opened and locked for this adapter's life.
    state: PathBuf,
    lock: StateDirLock,
}

impl FileBoardTokens {
    /// The tokens in `state` (made mode 0700, locked until this is dropped).
    /// # Errors
    /// A bad state directory, one another server holds, or a bad file.
    pub fn open(state: &Path) -> Result<Self, String> {
        private_dir(state)?;
        let (state, lock) = (state.to_path_buf(), locked(state, SERVER_HOLDS)?);
        let (held, writing) = (RwLock::default(), Mutex::default());
        let tokens = Self {
            held,
            writing,
            state,
            lock,
        };
        let (held, stamp) = tokens.read().map_err(|error| error.to_string())?;
        let (held, writing) = (RwLock::new(Arc::new(held)), Mutex::new(stamp));
        Ok(Self {
            held,
            writing,
            ..tokens
        })
    }

    fn unavailable(&self, why: impl std::fmt::Display) -> TokenError {
        let path = self.state.join(TOKENS_FILE);
        TokenError::Unavailable(format!("the tokens file {}: {why}", path.display()))
    }

    /// `name` in the locked directory, opened never through a symlink.
    fn open_at(&self, name: &CStr, flags: libc::c_int) -> io::Result<fs::File> {
        let flags = flags | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: a directory descriptor this adapter owns, a NUL-ended name.
        let fd = unsafe { libc::openat(self.lock.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        match fd >= 0 {
            // SAFETY: `fd` was just opened and nothing else owns it.
            true => Ok(unsafe { fs::File::from_raw_fd(fd) }),
            false => Err(io::Error::last_os_error()),
        }
    }

    /// The file, opened to read; none while it is missing.
    fn opened(&self) -> Result<Option<(fs::File, fs::Metadata)>, TokenError> {
        let file = match self.open_at(TOKENS_NAME, libc::O_RDONLY) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(self.unavailable(error)),
        };
        let meta = file.metadata().map_err(|error| self.unavailable(error))?;
        Ok(Some((file, meta)))
    }

    /// The file's stamp: none while it is missing.
    fn stamp(&self) -> Result<Option<Stamp>, TokenError> {
        Ok(self.opened()?.map(|(_, meta)| stamp(&meta)))
    }

    /// The tokens the file holds and its stamp: none while it is missing.
    fn read(&self) -> Result<(Vec<Held>, Option<Stamp>), TokenError> {
        let Some((file, meta)) = self.opened()? else {
            return Ok((Vec::new(), None));
        };
        let mode = meta.permissions().mode() & 0o777;
        match meta.is_file() && meta.uid() == this_uid() && mode == 0o600 {
            true if meta.len() <= MAX_FILE_BYTES => {}
            true => return Err(self.unavailable("past 1 MiB")),
            false => return Err(self.unavailable("must be a file of this user's, mode 0600")),
        }
        let mut text = String::new();
        (file.take(MAX_FILE_BYTES).read_to_string(&mut text)).map_err(|e| self.unavailable(e))?;
        let stored: Vec<StoredToken> =
            serde_json::from_str(&text).map_err(|e| self.unavailable(e))?;
        let entry =
            |token| held(token).ok_or_else(|| self.unavailable("an entry is not as written"));
        Ok((
            stored.into_iter().map(entry).collect::<Result<_, _>>()?,
            Some(stamp(&meta)),
        ))
    }

    /// Writes `held` as the file: a temporary file renamed over it, both
    /// in the locked directory.
    fn store(&self, held: &[Held]) -> Result<(), TokenError> {
        let tokens = held.iter().map(|held| StoredToken {
            name: held.record.name.as_str().to_string(),
            sha256: held.digest.clone(),
            created: held.record.created.as_str().to_string(),
            replica: held.record.kind == TokenKind::Replica,
        });
        let text = serde_json::to_vec_pretty(&tokens.collect::<Vec<_>>());
        let text = text.map_err(|e| self.unavailable(e))?;
        self.write_through(TOKENS_NAME, &text)
    }

    /// Writes `text` as the file `name` in the locked directory: a
    /// temporary file renamed over it.
    fn write_through(&self, name: &CStr, text: &[u8]) -> Result<(), TokenError> {
        let temp = CString::new(format!(".{TOKENS_FILE}.{}.tmp", uuid::Uuid::new_v4()));
        let temp = temp.map_err(|error| self.unavailable(error))?;
        let dir = self.lock.as_raw_fd();
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL;
        let written = self.open_at(&temp, flags).and_then(|mut file| {
            file.write_all(text)?;
            file.sync_all()?;
            // SAFETY: names in the directory this adapter holds open.
            match unsafe { libc::renameat(dir, temp.as_ptr(), dir, name.as_ptr()) } {
                0 => Ok(()),
                _ => Err(io::Error::last_os_error()),
            }
        });
        match written {
            // The rename has landed; a failed directory sync loses nothing.
            Ok(()) => self.lock.sync_all().or(Ok(())),
            Err(error) => {
                // SAFETY: as above; a temporary file that is not there is fine.
                let _ = unsafe { libc::unlinkat(dir, temp.as_ptr(), 0) };
                Err(self.unavailable(error))
            }
        }
    }

    /// The digests kept of the sets taken from a primary: the last one,
    /// and while a set is being swapped in, the one it replaces too.
    fn copied(&self) -> Result<Vec<String>, TokenError> {
        let file = match self.open_at(FOLLOWED_NAME, libc::O_RDONLY | libc::O_NONBLOCK) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(self.unavailable(error)),
        };
        let mut text = String::new();
        (file.take(256).read_to_string(&mut text)).map_err(|error| self.unavailable(error))?;
        Ok(text.lines().map(str::to_string).collect())
    }

    /// Keeps `digests` as the sets taken from a primary.
    fn keep_copied(&self, digests: &[String]) -> Result<(), TokenError> {
        match self.copied()? == digests {
            true => Ok(()),
            false => self.write_through(
                FOLLOWED_NAME,
                format!("{}\n", digests.join("\n")).as_bytes(),
            ),
        }
    }

    /// Whether the directory at the state path is still the one locked
    /// (one moved away and replaced would let another server in).
    fn still_locked(&self) -> Result<(), TokenError> {
        let identity = |meta: fs::Metadata| (meta.dev(), meta.ino());
        let at_path = fs::symlink_metadata(&self.state).map(identity);
        match (at_path, self.lock.metadata().map(identity)) {
            (Ok(found), Ok(locked)) if found == locked => Ok(()),
            (Ok(_) | Err(_), _) => Err(TokenError::Unavailable(format!(
                "the state directory {} was moved or replaced while this server held it",
                self.state.display()
            ))),
        }
    }

    /// Runs `change` on a copy of the tokens, writes it through, then keeps
    /// it; a hand edit behind the server is refused, never overwritten.
    fn changing<T>(
        &self,
        change: impl FnOnce(&mut Vec<Held>) -> Result<T, TokenError>,
    ) -> Result<T, TokenError> {
        let mut written = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        self.still_locked()?;
        match self.stamp()? == *written {
            true => {}
            false => return Err(self.unavailable(CHANGED_BEHIND)),
        }
        let mut next = Vec::clone(&self.held());
        let changed = change(&mut next)?;
        self.store(&next)?;
        *self.held.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(next);
        *written = self.stamp()?;
        Ok(changed)
    }

    fn held(&self) -> Arc<Vec<Held>> {
        Arc::clone(&self.held.read().unwrap_or_else(PoisonError::into_inner))
    }
}

impl FileBoardTokens {
    /// Notes, then swaps in, the primary's set `rows` (a replace untrimmed).
    fn swapped_in(&self, rows: &[AdminToken]) -> Result<String, TokenError> {
        let names: std::collections::BTreeSet<_> = rows.iter().map(|row| &row.name).collect();
        let shaped = |row: &AdminToken| {
            held(StoredToken {
                name: row.name.as_str().to_string(),
                sha256: row.digest.clone(),
                created: row.created.as_str().to_string(),
                replica: row.kind == TokenKind::Replica,
            })
        };
        let next: Option<Vec<Held>> = rows.iter().map(shaped).collect();
        let next = match (next, names.len() == rows.len(), rows.len() <= MAX_TOKENS) {
            (Some(next), true, true) => next,
            (_, _, _) => {
                return Err(TokenError::Unavailable(
                    "the primary's tokens are not as a token is kept".into(),
                ));
            }
        };
        let mut now = self.export()?;
        let mut wanted = rows.to_vec();
        wanted.sort_by(|a, b| a.name.cmp(&b.name));
        now.sort_by(|a, b| a.name.cmp(&b.name));
        let (held, taken) = (set_digest(&now), set_digest(&wanted));
        // While the set is swapped, both it and the one it replaces (when
        // that was the primary's too) are noted: a crash between the two
        // writes leaves a set a restart knows as its primary's.
        match now == wanted {
            true => {}
            false => {
                let mut noted = vec![taken.clone()];
                if self.copied()?.contains(&held) {
                    noted.push(held);
                }
                self.keep_copied(&noted)?;
                self.changing(|held| {
                    *held = next;
                    Ok(())
                })?;
            }
        }
        Ok(taken)
    }

    /// Makes a token of `kind`, keeping its digest.
    fn added(
        &self,
        name: &Slug,
        created: &Timestamp,
        kind: TokenKind,
    ) -> Result<IssuedToken, TokenError> {
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
            let random = rand::rngs::OsRng.try_fill_bytes(&mut bytes);
            random
                .map_err(|e| TokenError::Unavailable(format!("no randomness for a token: {e}")))?;
            let secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            let record = TokenRecord {
                name: name.clone(),
                created: created.clone(),
                kind,
            };
            held.push(Held {
                record: record.clone(),
                digest: sha256(&secret),
            });
            Ok(IssuedToken { record, secret })
        })
    }
}

impl BoardTokens for FileBoardTokens {
    fn add(&self, name: &Slug, created: &Timestamp) -> Result<IssuedToken, TokenError> {
        self.added(name, created, TokenKind::Device)
    }

    fn add_replica(&self, name: &Slug, created: &Timestamp) -> Result<IssuedToken, TokenError> {
        self.added(name, created, TokenKind::Replica)
    }

    fn kind_of(&self, name: &Slug) -> Result<Option<TokenKind>, TokenError> {
        let held = self.held();
        let found = held.iter().find(|held| &held.record.name == name);
        Ok(found.map(|held| held.record.kind))
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
        let mut records: Vec<_> = self.held().iter().map(|held| held.record.clone()).collect();
        records.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(records)
    }

    fn verify(&self, presented: &str) -> Result<Option<Slug>, TokenError> {
        let presented = sha256(presented);
        // Every digest is compared whole, in constant time and with no early
        // end, so the time taken says nothing of which (if any) matched.
        let mut found = None;
        for held in self.held().iter() {
            if constant_time_eq(&held.digest, &presented) {
                found = Some(held.record.name.clone());
            }
        }
        Ok(found)
    }
}

/// The tokens as admin rows (B11): a primary gives them to its followers,
/// a follower holds its primary's alone, written through only when they
/// change.
impl AdminRows for FileBoardTokens {
    fn export(&self) -> Result<Vec<AdminToken>, TokenError> {
        let mut rows: Vec<AdminToken> = self.held().iter().map(row).collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rows)
    }

    fn replace(&self, rows: &[AdminToken]) -> Result<(), TokenError> {
        let taken = self.swapped_in(rows)?;
        self.keep_copied(&[taken])
    }
}

/// A follower's tokens (B11): the set held is its primary's when its digest
/// is the one kept as the last set taken from it.
impl FollowTokens for FileBoardTokens {
    fn followed(&self) -> Result<FollowedTokens, TokenError> {
        let rows = self.export()?;
        let copied = self.copied()?;
        Ok(FollowedTokens {
            held: rows.len(),
            copied: copied.contains(&set_digest(&rows)),
        })
    }

    fn set_aside(&self) -> Result<(), TokenError> {
        let mut kept = Vec::new();
        if let Some((file, _)) = self.opened()? {
            (file.take(MAX_FILE_BYTES).read_to_end(&mut kept))
                .map_err(|error| self.unavailable(error))?;
        }
        let kept = match kept.is_empty() {
            true => b"[]\n".to_vec(),
            false => kept,
        };
        self.still_locked()?;
        // Created new (never through a name taken, a link included): the
        // first free of `.pre-follow`, `-1` .. `-99`.
        let mut file = None;
        for n in 0..100 {
            let name = match n {
                0 => PRE_FOLLOW_FILE.to_owned(),
                n => format!("{PRE_FOLLOW_FILE}-{n}"),
            };
            let name = CString::new(name).map_err(|error| self.unavailable(error))?;
            match self.open_at(&name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL) {
                Ok(created) => {
                    file = Some(created);
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(self.unavailable(error)),
            }
        }
        let mut file = file.ok_or_else(|| {
            self.unavailable(format!("100 {PRE_FOLLOW_FILE} files are kept: move them"))
        })?;
        (file.write_all(&kept).and_then(|()| file.sync_all()))
            .map_err(|error| self.unavailable(error))?;
        self.lock
            .sync_all()
            .map_err(|error| self.unavailable(error))?;
        self.changing(|held| {
            held.clear();
            Ok(())
        })?;
        self.keep_copied(&[set_digest(&[])])
    }
}

/// The digest of a token set, as rows sorted by name.
fn set_digest(rows: &[AdminToken]) -> String {
    let mut lines: Vec<String> = rows
        .iter()
        .map(|row| {
            let (name, created) = (row.name.as_str(), row.created.as_str());
            format!("{name}\t{created}\t{}\t{}\n", row.kind.as_str(), row.digest)
        })
        .collect();
    lines.sort();
    sha256(&lines.concat())
}

/// A held token as an admin row.
fn row(held: &Held) -> AdminToken {
    AdminToken {
        name: held.record.name.clone(),
        created: held.record.created.clone(),
        kind: held.record.kind,
        digest: held.digest.clone(),
    }
}

/// A stored entry as a held token, when every field is as written.
fn held(token: StoredToken) -> Option<Held> {
    let hex = |byte: &u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte);
    let digest = (token.sha256.len() == 64 && token.sha256.as_bytes().iter().all(hex))
        .then_some(token.sha256)?;
    let name = Slug::parse(&token.name).ok()?;
    let created = Timestamp::parse(&token.created).ok()?;
    let kind = match token.replica {
        true => TokenKind::Replica,
        false => TokenKind::Device,
    };
    Some(Held {
        record: TokenRecord {
            name,
            created,
            kind,
        },
        digest,
    })
}

/// `text`'s SHA-256, as 64 lowercase hex digits.
fn sha256(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(text.as_bytes());
    let hex = digest
        .iter()
        .flat_map(|byte| [HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 15)]]);
    hex.map(char::from).collect()
}

#[cfg(test)]
#[path = "board_tokens_tests.rs"]
mod tests;
