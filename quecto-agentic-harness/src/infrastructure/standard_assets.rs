//! Embedded, versioned standard project assets and safe initialization.
//!
//! The catalog is compiled into the executable. It deliberately has no runtime
//! dependency on the source checkout: `quecto container init` can be run from
//! any directory and materializes only missing files.

#[cfg(not(unix))]
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use sha2::{Digest, Sha256};

pub const STANDARD_ASSET_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardAsset {
    pub path: &'static str,
    pub contents: &'static str,
    /// Unix mode requested when the asset is first published. Existing files
    /// are never chmod'ed, preserving user-owned project state.
    pub mode: u32,
}

const ASSETS: &[StandardAsset] = &[
    StandardAsset { path: "standard-container/Containerfile", contents: include_str!("../../assets/standard-container/Containerfile"), mode: 0o644 },
    StandardAsset { path: "standard-container/config.json", contents: include_str!("../../assets/standard-container/config.json"), mode: 0o600 },
    StandardAsset { path: "standard-container/scripts/runtime/create.sh", contents: include_str!("../../assets/standard-container/scripts/runtime/create.sh"), mode: 0o755 },
    StandardAsset { path: "standard-container/scripts/runtime/exec.sh", contents: include_str!("../../assets/standard-container/scripts/runtime/exec.sh"), mode: 0o755 },
    StandardAsset { path: "standard-container/scripts/runtime/inspect.sh", contents: include_str!("../../assets/standard-container/scripts/runtime/inspect.sh"), mode: 0o755 },
    StandardAsset { path: "standard-container/scripts/runtime/kill.sh", contents: include_str!("../../assets/standard-container/scripts/runtime/kill.sh"), mode: 0o755 },
];

/// Return the immutable catalog embedded in this executable.
pub fn standard_assets() -> &'static [StandardAsset] { ASSETS }

/// Stable manifest entries (path, byte length, SHA-256) for diagnostics/tests.
pub fn standard_asset_manifest() -> Vec<(&'static str, usize, String)> {
    ASSETS.iter().map(|asset| {
        let digest = Sha256::digest(asset.contents.as_bytes());
        (asset.path, asset.contents.len(), format!("{digest:x}"))
    }).collect()
}

/// Materialize missing standard assets below `project`.
pub fn materialize_standard_assets(project: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    let project = project.as_ref();
    materialize_standard_assets_for_root(project, project)
}

/// Materialize into `bundle`, while expanding generated configuration paths
/// relative to the owning project root. Publication is descriptor-relative on
/// Unix: every directory component is opened with `O_NOFOLLOW`, and each file
/// is linked into its already-open parent directory. Thus a concurrent rename
/// or symlink replacement cannot redirect a write outside the bundle.
pub fn materialize_standard_assets_for_root(
    bundle: &Path,
    project_root: &Path,
) -> io::Result<Vec<PathBuf>> {
    #[cfg(unix)]
    { materialize_unix(bundle, project_root) }
    #[cfg(not(unix))]
    { materialize_portable(bundle, project_root) }
}

fn expanded_contents(asset: &StandardAsset, project_root: &Path) -> io::Result<Vec<u8>> {
    if !asset.path.ends_with("config.json") {
        return Ok(asset.contents.as_bytes().to_vec());
    }
    // The placeholder is inside a JSON string. Serialize the replacement as a
    // JSON string and remove only its outer quotes; this escapes quotes,
    // backslashes, controls, and non-UTF-8 paths (via to_string_lossy).
    let project = project_root.to_string_lossy().into_owned();
    let encoded = serde_json::to_string(&project)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("cannot encode project path as JSON: {error}")))?;
    let replacement = encoded
        .strip_prefix('"').and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "JSON string serializer returned an invalid string"))?;
    let expanded = asset.contents.replace("@PROJECT@", replacement);
    // Keep the embedded asset contract explicit: never publish malformed JSON.
    serde_json::from_str::<serde_json::Value>(&expanded)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("expanded standard config is invalid JSON: {error}")))?;
    Ok(expanded.into_bytes())
}

#[cfg(unix)]
fn materialize_unix(bundle: &Path, project_root: &Path) -> io::Result<Vec<PathBuf>> {
    use std::os::fd::AsRawFd;

    let bundle_fd = open_directory_chain(bundle)?;
    let project_root = project_root.canonicalize().unwrap_or_else(|_| project_root.to_path_buf());
    let mut created = Vec::new();
    for (index, asset) in ASSETS.iter().enumerate() {
        let relative = safe_relative_path(asset.path)?;
        let parent = relative.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no parent"))?;
        let parent_fd = open_relative_directory_chain(bundle_fd.as_raw_fd(), parent)?;
        let name = relative.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no name"))?;
        let bytes = expanded_contents(asset, &project_root)?;
        if publish_new_atomic_at(parent_fd.as_raw_fd(), name, &bytes, asset.mode, index)? {
            created.push(bundle.join(&relative));
        }
    }
    Ok(created)
}

#[cfg(unix)]
fn open_directory_chain(path: &Path) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::ffi::CString;

    let (start, components): (RawFd, Vec<_>) = if path.is_absolute() {
        (open_dir_fd(libc::AT_FDCWD, std::ffi::OsStr::new("/"))?, path.components().collect())
    } else {
        (open_dir_fd(libc::AT_FDCWD, std::ffi::OsStr::new("."))?, path.components().collect())
    };
    // `start` is owned by the returned File. Components are accepted only from
    // the allowlist of Normal/CurDir; ParentDir would escape the descriptor.
    // SAFETY: `start` came from `open_dir_fd` and is an owned, valid directory
    // descriptor; transferring exactly one ownership to File prevents leaks.
    // SAFETY: start is an owned valid directory descriptor transferred once.
    let mut current = unsafe { std::fs::File::from_raw_fd(start) };
    for component in components {
        let Component::Normal(name) = component else {
            if matches!(component, Component::RootDir | Component::CurDir) { continue; }
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "directory path contains an unsafe component"));
        };
        let name = CString::new(name.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory name contains NUL"))?;
        let name_os = std::ffi::OsStr::from_bytes(name.as_bytes());
        let next = match open_dir_fd(current.as_raw_fd(), name_os) {
            Ok(fd) => fd,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if mkdir_at(current.as_raw_fd(), name.as_c_str(), 0o755).is_err() {
                    // A concurrent creator may have won; the no-follow open
                    // below is the authority and rejects symlink substitution.
                }
                open_dir_fd(current.as_raw_fd(), name_os)?
            }
            Err(error) => return Err(error),
        };
        // SAFETY: `next` is returned by `open_dir_fd` with ownership and is
        // transferred once into File for automatic close.
        // SAFETY: `next` is an owned descriptor from open_dir_fd and is transferred once.
        current = unsafe { std::fs::File::from_raw_fd(next) };
    }
    Ok(current)
}

#[cfg(unix)]
fn open_relative_directory_chain(parent: std::os::fd::RawFd, path: &Path) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::ffi::CString;
    let mut current = {
        // SAFETY: `parent` is an open directory descriptor owned by the caller;
        // dup creates an independent descriptor or returns a checked error.
        // SAFETY: parent is an open directory descriptor; dup validates it and returns ownership.
        let fd = unsafe { libc::dup(parent) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        // SAFETY: `fd` is the newly-owned descriptor returned by dup and is
        // transferred exactly once to File.
        // SAFETY: fd is newly owned from dup and transferred exactly once.
        unsafe { std::fs::File::from_raw_fd(fd) }
    };
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "asset parent contains an unsafe component"));
        };
        let name = CString::new(name.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory name contains NUL"))?;
        let name_os = std::ffi::OsStr::from_bytes(name.as_bytes());
        let next = match open_dir_fd(current.as_raw_fd(), name_os) {
            Ok(fd) => fd,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let _ = mkdir_at(current.as_raw_fd(), name.as_c_str(), 0o755);
                open_dir_fd(current.as_raw_fd(), name_os)?
            }
            Err(error) => return Err(error),
        };
        // SAFETY: `next` is returned by `open_dir_fd` with ownership and is
        // transferred once into File for automatic close.
        // SAFETY: `next` is an owned descriptor from open_dir_fd and is transferred once.
        current = unsafe { std::fs::File::from_raw_fd(next) };
    }
    Ok(current)
}

#[cfg(unix)]
fn open_dir_fd(parent: std::os::fd::RawFd, name: &std::ffi::OsStr) -> io::Result<std::os::fd::RawFd> {
    use std::os::unix::ffi::OsStrExt;
    use std::ffi::CString;
    let name = CString::new(name.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    // SAFETY: `parent` is an open directory fd and `name` is a checked NUL-free
    // path; flags prevent symlink traversal and the returned fd is checked.
    // SAFETY: pointers are NUL-terminated C strings and flags enforce directory/no-follow.
    let fd = unsafe { libc::openat(parent, name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(fd) }
}

#[cfg(unix)]
fn mkdir_at(parent: std::os::fd::RawFd, name: &std::ffi::CStr, mode: u32) -> io::Result<()> {
    // SAFETY: parent is an open directory descriptor and name is NUL-terminated.
    let result = unsafe { libc::mkdirat(parent, name.as_ptr(), mode) };
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

#[cfg(unix)]
fn publish_new_atomic_at(parent: std::os::fd::RawFd, name: &std::ffi::OsStr, bytes: &[u8], mode: u32, index: usize) -> io::Result<bool> {
    use std::ffi::CString;
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);

    let name = CString::new(name.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "asset name contains NUL"))?;
    // Existing symlinks are an explicit refusal. The fstatat check is only a
    // diagnostic; linkat below remains the race-safe publication authority.
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat points to valid MaybeUninit storage; parent/name are validated.
    let stat_result = unsafe { libc::fstatat(parent, name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
    if stat_result == 0 {
        // SAFETY: fstatat returned success, so the kernel initialized stat.
        let stat = unsafe { stat.assume_init() };
        if (stat.st_mode & libc::S_IFMT) == libc::S_IFLNK {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "refusing symlink asset destination"));
        }
        return Ok(false);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(libc::ENOENT) { return Err(error); }

    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let temporary = CString::new(format!(".{}.quecto-{}-{}-{}", name.to_string_lossy(), std::process::id(), index, serial)).expect("generated temporary name has no NUL");
    // SAFETY: temporary is NUL-terminated and O_EXCL/O_NOFOLLOW prevent substitution.
    let temp_fd: RawFd = unsafe { libc::openat(parent, temporary.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600) };
    if temp_fd < 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: temp_fd is a newly-owned successful descriptor transferred once.
    let mut file = unsafe { std::fs::File::from_raw_fd(temp_fd) };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        // SAFETY: file owns a valid descriptor for the temporary file.
        let chmod = unsafe { libc::fchmod(file.as_raw_fd(), mode) };
        if chmod < 0 { return Err(io::Error::last_os_error()); }
        // SAFETY: both names are validated NUL-terminated strings and parent is open.
        let linked = unsafe { libc::linkat(parent, temporary.as_ptr(), parent, name.as_ptr(), 0) };
        if linked == 0 { Ok(true) }
        else if io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) { Ok(false) }
        else { Err(io::Error::last_os_error()) }
    })();
    drop(file);
    // SAFETY: temporary is the file created above in the same open directory.
    unsafe { libc::unlinkat(parent, temporary.as_ptr(), 0); }
    result
}

#[cfg(not(unix))]
fn materialize_portable(bundle: &Path, project_root: &Path) -> io::Result<Vec<PathBuf>> {
    ensure_directory_chain(bundle)?;
    let project_root = project_root.canonicalize().unwrap_or_else(|_| project_root.to_path_buf());
    let mut created = Vec::new();
    for (index, asset) in ASSETS.iter().enumerate() {
        let relative = safe_relative_path(asset.path)?;
        let destination = bundle.join(relative);
        reject_symlink_or_existing(&destination)?;
        let parent = destination.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no parent"))?;
        ensure_directory_chain(parent)?;
        let contents = expanded_contents(asset, &project_root)?;
        if publish_new_atomic(&destination, &contents, asset.mode, index)? { created.push(destination); }
    }
    Ok(created)
}

#[cfg(not(unix))]
fn ensure_directory_chain(path: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("refusing symlink directory: {}", current.display()))),
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("asset parent is not a directory: {}", current.display()))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn reject_symlink_or_existing(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("refusing symlink asset destination: {}", path.display()))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
fn publish_new_atomic(destination: &Path, bytes: &[u8], mode: u32, index: usize) -> io::Result<bool> {
    let parent = destination.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no parent"))?;
    let name = destination.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no name"))?.to_string_lossy();
    let temporary = parent.join(format!(".{name}.quecto-{index}-{}", std::process::id()));
    let mut file = match fs::OpenOptions::new().write(true).create_new(true).open(&temporary) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("temporary asset path already exists: {}", temporary.display()))),
        Err(error) => return Err(error),
    };
    let result = (|| {
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::set_permissions(&temporary, fs::Permissions::from_readonly(false))?;
        match fs::hard_link(&temporary, destination) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    })();
    drop(file);
    let _ = fs::remove_file(&temporary);
    let _ = mode;
    result
}

fn safe_relative_path(path: &str) -> io::Result<PathBuf> {
    let candidate = Path::new(path);
    let components: Vec<_> = candidate.components().collect();
    let valid = !components.is_empty() && components.iter().all(|component| matches!(component, Component::Normal(_)));
    if valid { Ok(components.into_iter().collect()) } else { Err(io::Error::new(io::ErrorKind::InvalidInput, "standard asset path is not relative and safe")) }
}

#[cfg(test)]
#[path = "standard_assets_tests.rs"]
mod standard_assets_tests;
