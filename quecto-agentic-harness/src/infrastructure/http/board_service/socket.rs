//! The board service's sockets (epic #2705 B4, B5): a Unix socket bound
//! mode 0600 in a directory of mode 0700, and a TCP address bound as given.
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// This process's effective uid: the one a server lets in.
pub fn this_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Why a socket was not bound: a server answers on it, or why not.
#[derive(Debug)]
pub enum BindError {
    AlreadyServing(PathBuf),
    Refused(String),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyServing(path) => {
                write!(f, "a server is already serving on {}", path.display())
            }
            Self::Refused(why) => f.write_str(why),
        }
    }
}

/// A bound listener: the Unix socket, or a TCP address.
#[derive(Debug)]
pub enum Listener {
    Unix(std::os::unix::net::UnixListener),
    Tcp(std::net::TcpListener),
}

/// Binds TCP on `address` (its caller holds the allowlist): the listener
/// and the address bound.
/// # Errors
/// The bind failed.
pub fn bind_tcp(
    address: std::net::SocketAddr,
) -> Result<(std::net::TcpListener, std::net::SocketAddr), BindError> {
    let refused = |error: io::Error| BindError::Refused(format!("{address}: {error}"));
    let listener = std::net::TcpListener::bind(address).map_err(refused)?;
    let bound = listener.local_addr().map_err(refused)?;
    Ok((listener, bound))
}

/// Binds a Unix socket at `path`, mode 0600, in a directory of mode 0700
/// (made if missing). Only a stale socket of this user's is replaced.
/// # Errors
/// A relative path, a shared directory, a path holding anything but a
/// stale socket of this user's, or the bind.
pub fn bind(path: &Path) -> Result<std::os::unix::net::UnixListener, BindError> {
    let shown = path.display();
    let relative = || BindError::Refused(format!("the socket path {shown} is not absolute"));
    private_dir(
        path.parent()
            .filter(|_| path.is_absolute())
            .ok_or_else(relative)?,
    )?;
    let io = |error| BindError::Refused(format!("{shown}: {error}"));
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == this_uid() => {
            match std::os::unix::net::UnixStream::connect(path) {
                Ok(_) => return Err(BindError::AlreadyServing(path.to_path_buf())),
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    fs::remove_file(path).map_err(io)?;
                }
                Err(error) => return Err(io(error)),
            }
        }
        Ok(_) => {
            let why =
                format!("{shown} exists and is not a socket of this user's: it is left as it is");
            return Err(BindError::Refused(why));
        }
        Err(error) => return Err(io(error)),
    }
    let listener = std::os::unix::net::UnixListener::bind(path).map_err(io)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io)?;
    match fs::symlink_metadata(path).map_err(io)?.permissions().mode() & 0o777 {
        0o600 => Ok(listener),
        _ => Err(io(io::Error::other("the socket did not take mode 0600"))),
    }
}

/// `dir` as a directory of mode 0700 of this user's, made so if missing.
fn private_dir(dir: &Path) -> Result<(), BindError> {
    let refused = |why: &str| BindError::Refused(format!("{}: {why}", dir.display()));
    let io = |error: io::Error| refused(&error.to_string());
    match fs::symlink_metadata(dir).map_err(|error| error.kind()) {
        Ok(_) => {}
        Err(io::ErrorKind::NotFound) => match fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io)?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io(error)),
        },
        Err(kind) => return Err(io(kind.into())),
    }
    let meta = fs::symlink_metadata(dir).map_err(io)?;
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
