//! Owner-only authority directory and the singleton lock that must precede
//! socket binding (ADR-0026).
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

const CLIENT_SUBDIR: &str = "client";
const CLIENT_SOCKET: &str = "admission.sock";
const ADMIN_SOCKET: &str = "admin.sock";
const LOCK_FILE: &str = "authority.lock";
const JOURNAL_FILE: &str = "journal.json";
const ROOT_TOKEN_FILE: &str = "root.token";

/// Layout of one authority's private directory. The `client/` subdirectory is
/// the only part a container child ever sees (mounted by path); administration
/// and the journal stay outside it.
#[derive(Debug, Clone)]
pub struct AuthorityDirectory {
    root: PathBuf,
}

impl AuthorityDirectory {
    /// Create or validate the directory: owned by the current user and
    /// inaccessible to group/other. A more permissive directory is refused
    /// rather than silently tightened, so an operator sees the misconfiguration.
    pub fn open(path: &Path) -> io::Result<Self> {
        let dir = Self {
            root: path.to_path_buf(),
        };
        ensure_private_dir(&dir.root)?;
        ensure_private_dir(&dir.client_dir())?;
        Ok(dir)
    }

    /// Validate an authority directory that must already exist (client side):
    /// same ownership and mode checks as `open`, but nothing is created.
    pub fn existing(path: &Path) -> io::Result<Self> {
        let dir = Self {
            root: path.to_path_buf(),
        };
        for candidate in [&dir.root, &dir.client_dir()] {
            let meta = fs::symlink_metadata(candidate)?;
            if !meta.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} is not a directory", candidate.display()),
                ));
            }
            check_private(candidate, &meta)?;
        }
        Ok(dir)
    }

    pub fn path(&self) -> &Path {
        &self.root
    }
    pub fn client_dir(&self) -> PathBuf {
        self.root.join(CLIENT_SUBDIR)
    }
    pub fn client_socket(&self) -> PathBuf {
        self.client_dir().join(CLIENT_SOCKET)
    }
    pub fn admin_socket(&self) -> PathBuf {
        self.root.join(ADMIN_SOCKET)
    }
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }
    pub fn journal_path(&self) -> PathBuf {
        self.root.join(JOURNAL_FILE)
    }
    /// Owner token that mints roots; outside `client/`, so a process that only
    /// sees the mounted client directory can bind children but not promote.
    pub fn root_token_path(&self) -> PathBuf {
        self.root.join(ROOT_TOKEN_FILE)
    }
}

fn ensure_private_dir(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a directory", path.display()),
                ));
            }
            check_private(path, &meta)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            check_private(path, &fs::symlink_metadata(path)?)
        }
        Err(e) => Err(e),
    }
}

fn current_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

fn check_private(path: &Path, meta: &fs::Metadata) -> io::Result<()> {
    if meta.uid() != current_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not owned by the current user", path.display()),
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} must be mode 0700 (owner only); refusing a shared directory",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// Exclusive `flock` on the directory's lock file. Held for the lifetime of the
/// value; process death releases it. Acquire this before binding any socket.
#[derive(Debug)]
pub struct SingletonLock {
    _file: fs::File,
}

impl SingletonLock {
    // `File::try_lock` stabilized in 1.89; see `session_ownership.rs` for the
    // toolchain-floor note shared with the #1460 locking work.
    #[expect(clippy::incompatible_msrv)]
    pub fn acquire(dir: &AuthorityDirectory) -> io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(dir.lock_path())?;
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "another admission authority owns {}",
                        dir.lock_path().display()
                    ),
                ));
            }
            Err(fs::TryLockError::Error(e)) => return Err(e),
        }
        Ok(Self { _file: file })
    }
}
