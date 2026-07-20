use crate::domain::error::DomainError;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

fn is_linux_deleted_exe_path(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().ends_with(" (deleted)")
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn resolve_argv0_from_path_var(argv0: &OsStr, path_var: Option<OsString>) -> Option<PathBuf> {
    let argv0_path = Path::new(argv0);
    if argv0_path.components().count() > 1 && is_executable_file(argv0_path) {
        return Some(argv0_path.to_path_buf());
    }

    let path_name = argv0_path.file_name().unwrap_or(argv0);
    for dir in std::env::split_paths(&path_var?) {
        let candidate = dir.join(path_name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub(super) fn resolve_child_binary_with<F, I, P>(
    current_exe: F,
    mut args: I,
    path_var: P,
) -> Result<PathBuf, DomainError>
where
    F: FnOnce() -> std::io::Result<PathBuf>,
    I: Iterator<Item = OsString>,
    P: FnOnce() -> Option<OsString>,
{
    let current = current_exe();
    if let Ok(path) = &current {
        if !is_linux_deleted_exe_path(path) {
            return Ok(path.clone());
        }
        tracing::warn!(
            path = %path.display(),
            "current executable path is marked deleted; re-resolving child binary from argv[0]"
        );
    }

    if let Some(argv0) = args.next() {
        if let Some(path) = resolve_argv0_from_path_var(&argv0, path_var()) {
            tracing::warn!(
                path = %path.display(),
                "using re-resolved argv[0] for spawned child binary"
            );
            return Ok(path);
        }
    }

    match current {
        Ok(path) => Err(DomainError::Tool(format!(
            "cannot find usable quecto binary: current_exe() returned deleted path {} and argv[0] could not be resolved via PATH",
            path.display()
        ))),
        Err(e) => Err(DomainError::Tool(format!(
            "cannot find quecto binary: current_exe() failed ({e}) and argv[0] could not be resolved via PATH"
        ))),
    }
}

pub(super) fn resolve_child_binary() -> Result<PathBuf, DomainError> {
    resolve_child_binary_with(std::env::current_exe, std::env::args_os(), || {
        std::env::var_os("PATH")
    })
}

#[cfg(test)]
#[path = "spawn_binary_tests.rs"]
mod tests;
