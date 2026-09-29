//! The shared checkout's paths (#2275): `CheckoutPaths` over the
//! filesystem, with Python's non-strict `Path.resolve()`.
//!
//! Python's board normalises a reserved path as
//! `str((root / path).resolve().relative_to(root))`, `root` being the
//! checkout resolved. `resolve()` is `os.path.realpath(strict=False)`,
//! which walks the path one component at a time: an existing symlink is
//! followed (its target's components pushed back onto the walk, an
//! absolute target restarting from `/`), a component that cannot be
//! `lstat`ed (missing, under a regular file, in a symlink loop) is kept as
//! written, and `..` drops the last component of what is resolved so far.
//! `std::fs::canonicalize` is strict (it fails on a missing path), so the
//! walk is ported here, component for component, as CPython 3.13 and 3.14
//! write it.
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use crate::application::swarm::ports::CheckoutPaths;
use crate::domain::swarm::BoardError;

/// The checkout at `root`, resolved on every call as Python resolves it.
#[derive(Clone, Debug)]
pub struct ResolvedCheckout {
    root: PathBuf,
}

impl ResolvedCheckout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl CheckoutPaths for ResolvedCheckout {
    fn normalize(&self, path: &str) -> Result<String, BoardError> {
        let escape = || BoardError::new("file must resolve inside the shared checkout");
        // `lstat` of a path holding NUL raises `ValueError` in Python, which
        // the board reports as an escape.
        if path.contains('\0') {
            return Err(escape());
        }
        let root = realpath(self.root.as_os_str().as_bytes()).map_err(|_| escape())?;
        let joined = if path.starts_with('/') {
            path.as_bytes().to_vec()
        } else {
            let mut joined = root.clone();
            joined.push(b'/');
            joined.extend_from_slice(path.as_bytes());
            joined
        };
        let resolved = realpath(&joined).map_err(|_| escape())?;
        let relative = relative_to(&resolved, &root).ok_or_else(escape)?;
        // A name that is not UTF-8 (a worker's symlink can make one) is
        // refused as an escape, where Python's surrogate-escaped text makes
        // its `sqlite3` raise: `non_utf8_resolved_path_is_refused`.
        String::from_utf8(relative).map_err(|_| escape())
    }
}

/// `os.path.realpath(filename, strict=False)` for an absolute or relative
/// `filename` (relative to the working directory, as Python's is).
fn realpath(filename: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut path: Vec<u8> = if filename.starts_with(b"/") {
        b"/".to_vec()
    } else {
        std::env::current_dir()?.into_os_string().into_vec()
    };
    // The unresolved components, last first; `None` marks a symlink whose
    // target is resolved once the entries above it are consumed, and is
    // followed by that symlink's path.
    let mut rest: Vec<Option<Vec<u8>>> = split_reversed(filename);
    let mut part_count = rest.len();
    let mut seen: HashMap<Vec<u8>, Option<Vec<u8>>> = HashMap::new();
    while part_count > 0 {
        let Some(entry) = rest.pop() else {
            break;
        };
        let Some(name) = entry else {
            // A resolved symlink's target: record it for the symlink's path.
            if let Some(Some(link)) = rest.pop() {
                seen.insert(link, Some(path.clone()));
            }
            continue;
        };
        part_count -= 1;
        if name.is_empty() || name == b"." {
            continue;
        }
        if name == b".." {
            path = parent(&path);
            continue;
        }
        let newpath = child(&path, &name);
        match std::fs::symlink_metadata(OsStr::from_bytes(&newpath)) {
            Ok(metadata) if metadata.file_type().is_symlink() => {}
            // Not a symlink, or not `lstat`-able: kept as written.
            Ok(_) | Err(_) => {
                path = newpath;
                continue;
            }
        }
        if let Some(cached) = seen.get(&newpath) {
            // Seen before: resolved already, or a loop kept as written.
            path = cached.clone().unwrap_or(newpath);
            continue;
        }
        let Ok(target) = std::fs::read_link(OsStr::from_bytes(&newpath)) else {
            path = newpath;
            continue;
        };
        let target = OsString::from(target).into_vec();
        if target.starts_with(b"/") {
            path = b"/".to_vec();
        }
        seen.insert(newpath.clone(), None);
        rest.push(Some(newpath));
        rest.push(None);
        let parts = split_reversed(&target);
        part_count += parts.len();
        rest.extend(parts);
    }
    debug_assert!(path.starts_with(b"/"), "the resolved path is absolute");
    Ok(path)
}

/// `text.split('/')[::-1]`, each part an entry of the walk.
fn split_reversed(text: &[u8]) -> Vec<Option<Vec<u8>>> {
    text.split(|&byte| byte == b'/')
        .rev()
        .map(|part| Some(part.to_vec()))
        .collect()
}

/// `path[:path.rindex('/')] or '/'`.
fn parent(path: &[u8]) -> Vec<u8> {
    match path.iter().rposition(|&byte| byte == b'/') {
        Some(0) | None => b"/".to_vec(),
        Some(index) => path[..index].to_vec(),
    }
}

/// `path + name`, or `path + '/' + name` below the root.
fn child(path: &[u8], name: &[u8]) -> Vec<u8> {
    let mut joined = path.to_vec();
    if path != b"/" {
        joined.push(b'/');
    }
    joined.extend_from_slice(name);
    joined
}

/// `PurePath(resolved).relative_to(root)` as text: `resolved` below (or
/// at) `root` by whole components, or `None`. Both are `realpath`
/// results: absolute, without `.`, `..` or empty components.
fn relative_to(resolved: &[u8], root: &[u8]) -> Option<Vec<u8>> {
    let resolved = Path::new(OsStr::from_bytes(resolved));
    let root = Path::new(OsStr::from_bytes(root));
    let relative = resolved.strip_prefix(root).ok()?;
    let text = relative.as_os_str().as_bytes();
    Some(if text.is_empty() {
        b".".to_vec()
    } else {
        text.to_vec()
    })
}

#[cfg(test)]
#[path = "checkout_paths_tests.rs"]
mod tests;
