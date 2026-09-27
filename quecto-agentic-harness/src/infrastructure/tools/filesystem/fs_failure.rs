// Why a filesystem tool could not do what it was asked, in words the model
// can act on (#2189): every failure names the path, gives a plain reason and
// a next step. read, ls and edit share it, so their refusals read alike
// (#2193's shape): an `is_error` result with no "tool error" prefix.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::domain::tool::ToolResult;
use crate::infrastructure::file_replace::{
    FileState, InPlace, WriteFailure, Written, replace_contents,
};
use crate::infrastructure::tools::truncate::format_size;

use super::shell_escape_single;

/// A refusal: the tool did nothing, and the text says why and what next.
pub(super) fn refused(content: String) -> ToolResult {
    ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

/// What the tool was doing, so the reason and the next step fit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    Read,
    List,
    Edit,
}

impl Access {
    fn verb(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::List => "list",
            Self::Edit => "edit",
        }
    }
}

/// The failures that are explained in their own words; anything else is
/// [`Cause::Other`] and keeps the system's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    NotFound,
    IsADirectory,
    NotADirectory,
    LinkLoop,
    PermissionDenied,
    Other,
}

impl Cause {
    fn of(error: &std::io::Error) -> Self {
        // `ErrorKind::FilesystemLoop` is not stable: the loop is ELOOP.
        match (error.raw_os_error(), error.kind()) {
            (Some(libc::ELOOP), _) => Self::LinkLoop,
            (_, ErrorKind::NotFound) => Self::NotFound,
            (_, ErrorKind::IsADirectory) => Self::IsADirectory,
            (_, ErrorKind::NotADirectory) => Self::NotADirectory,
            (_, ErrorKind::PermissionDenied) => Self::PermissionDenied,
            (_, _) => Self::Other,
        }
    }
}

/// Why opening, reading or listing `path` (resolved: `full_path`) failed.
pub(super) async fn explain(
    access: Access,
    full_path: &Path,
    path: &str,
    error: &std::io::Error,
) -> String {
    let hint = shell_escape_single(path);
    let verb = access.verb();
    match Cause::of(error) {
        Cause::NotFound => match dangling_link_target(full_path).await {
            Some(target) => dangling(access, path, &target),
            None => not_found(access, full_path, path),
        },
        Cause::IsADirectory => match access {
            Access::Edit => format!(
                "{path} is a directory; edit changes a file. List it with ls to find the file."
            ),
            Access::Read | Access::List => {
                format!("{path} is a directory, not a file; list it with ls.")
            }
        },
        Cause::NotADirectory => match (access, is_file(full_path).await) {
            (Access::List, true) => {
                format!("{path} is a file, not a directory; read it with read.")
            }
            (Access::List | Access::Read | Access::Edit, _) => format!(
                "cannot {verb} {path}: a part of the path before its last name is a file, \
                 not a directory. Check the path with ls."
            ),
        },
        Cause::LinkLoop => match (
            walk_links(full_path).await,
            tokio::fs::read_link(full_path).await,
        ) {
            (LinkChain::Cycle, Ok(target)) => format!(
                "{path} is a symbolic link in a loop: it points to {}, and following it \
                 leads back to itself. Check it with bash, e.g. ls -l {hint}",
                target.display()
            ),
            (LinkChain::TooLong, _) => format!(
                "{path} is a chain of more than {SYMLOOP_MAX} symbolic links, which the system \
                 does not follow. Point it at its target directly; check it with bash, e.g. \
                 ls -l {hint}"
            ),
            (LinkChain::Cycle | LinkChain::Other, _) => format!(
                "cannot {verb} {path}: a directory in the path is a symbolic link that leads \
                 back to itself, or a chain of more than {SYMLOOP_MAX} links. Check its parts \
                 with bash, e.g. ls -l {hint}"
            ),
        },
        Cause::PermissionDenied => {
            let ls = match access {
                Access::List => "ls -ld",
                Access::Read | Access::Edit => "ls -l",
            };
            format!(
                "permission denied: cannot {verb} {path}. Check its permissions with bash, \
                 e.g. {ls} {hint}"
            )
        }
        Cause::Other => explain_other(access, path, error),
    }
}

/// A failure with no words of its own: the path, the system's text, and
/// where to look.
pub(super) fn explain_other(access: Access, path: &str, error: &std::io::Error) -> String {
    format!(
        "cannot {} {path}: {error}. Check the path with ls.",
        access.verb()
    )
}

fn dangling(access: Access, path: &str, target: &Path) -> String {
    let next = match access {
        Access::Read => "Read the file it should point to, or fix the link.",
        Access::List => "List the directory it should point to.",
        Access::Edit => "Edit the file it should point to, or use write to create it.",
    };
    format!(
        "{path} is a symbolic link to {}, which does not exist. {next}",
        target.display()
    )
}

/// Nothing has the name: say where it was looked for (when the path was
/// relative) and which directory to list instead.
fn not_found(access: Access, full_path: &Path, path: &str) -> String {
    let looked = match Path::new(path).is_absolute() {
        true => String::new(),
        false => format!(" (looked for {})", full_path.display()),
    };
    let parent = match Path::new(path).parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.display().to_string(),
        Some(_) | None => ".".to_string(),
    };
    match access {
        Access::Edit => format!(
            "file not found: {path}{looked}. edit changes an existing file; use write to \
             create a new one."
        ),
        Access::Read => {
            format!("file not found: {path}{looked}. Check the path, or list {parent} with ls.")
        }
        Access::List => format!(
            "directory not found: {path}{looked}. Check the path, or list {parent} with ls."
        ),
    }
}

async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|meta| meta.is_file())
}

/// Why writing `path` failed, and what that left (#2243): a file is
/// replaced whole or not at all, except one written in place (it has hard
/// links, its directory takes no new file, the system refused the rename,
/// or its owner could not be given to a new file), which may be cut short.
pub(super) fn write_failure(path: &str, failure: &WriteFailure) -> String {
    let (text, names_leftover) = failure_text(path, failure);
    match (&failure.leftover, names_leftover) {
        (Some(temp), false) => format!(
            "{text} Its temporary file {} could not be removed: delete it.",
            temp.display()
        ),
        (Some(_), true) | (None, _) => text,
    }
}

/// The words for [`write_failure`], and whether they name the leftover
/// temporary file themselves.
fn failure_text(path: &str, failure: &WriteFailure) -> (String, bool) {
    let hint = shell_escape_single(path);
    let error = &failure.error;
    let text = match (failure.file, error.kind()) {
        (FileState::Unknown, _) => format!(
            "writing {path} stopped on an internal error ({error}), so whether it was written \
             is not known. Read it to check before writing it again."
        ),
        (FileState::Unchanged, ErrorKind::NotFound) => format!(
            "{path} was removed while it was being written, so nothing was written. Check the \
             path with ls, and write it again if it is still wanted."
        ),
        (FileState::Unchanged, ErrorKind::PermissionDenied) => format!(
            "permission denied: cannot write {path}, so nothing was written. Check its \
             permissions with bash, e.g. ls -l {hint}"
        ),
        (FileState::Absent, ErrorKind::PermissionDenied) => {
            let dir = match Path::new(path).parent() {
                Some(dir) if !dir.as_os_str().is_empty() => dir.display().to_string(),
                Some(_) | None => ".".to_string(),
            };
            format!(
                "permission denied: cannot create {path} in its directory, so nothing was \
                 written. Check the directory's permissions with bash, e.g. ls -ld {}",
                shell_escape_single(&dir)
            )
        }
        (
            FileState::Unchanged | FileState::Absent,
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded,
        ) => {
            let state = match failure.file {
                FileState::Absent => "no file was created",
                FileState::Unchanged | FileState::MayBeCutShort | FileState::Unknown => {
                    "the file is as it was"
                }
            };
            format!(
                "writing {path} failed: the disk or quota is full, so nothing was written: \
                 {state}. Free space and retry."
            )
        }
        (FileState::Unchanged | FileState::Absent, ErrorKind::ReadOnlyFilesystem) => {
            format!("cannot write {path}: its file system is read-only, so nothing was written.")
        }
        (FileState::Unchanged, ErrorKind::IsADirectory) => {
            format!("{path} is a directory, so nothing was written. Give the path of a file.")
        }
        (FileState::Unchanged, _) => format!(
            "writing {path} failed: {error}. Nothing was written: the file is as it was \
             (it is replaced whole or not at all)."
        ),
        (FileState::Absent, _) => match &failure.leftover {
            Some(temp) => {
                let text = format!(
                    "creating {path} failed: {error}. No file was created at {path}; its \
                     temporary file {} could not be removed: delete it.",
                    temp.display()
                );
                return (text, true);
            }
            None => format!(
                "creating {path} failed: {error}. Nothing was created. Check the path with ls."
            ),
        },
        (FileState::MayBeCutShort, _) => format!(
            "writing {path} failed: {error}. It was being written in place (not atomically), \
             so it may now be cut short or a mix of old and new text; restore it (e.g. git \
             checkout -- {hint}) or rewrite it with write."
        ),
    };
    (text, false)
}

/// A note for a write that was not atomic, to follow the tool's result.
pub(super) fn written_note(path: &str, written: Written) -> Option<String> {
    let (why, leftover) = match written {
        Written::Replaced | Written::Created => return None,
        Written::InPlace(InPlace::HardLinks(links)) => (
            format!("{path} has {links} hard links, so it was written in place to keep them"),
            None,
        ),
        Written::InPlace(InPlace::RenameRefused { leftover }) => (
            format!(
                "the system refused to replace {path} by a rename (for example a mount point \
                 or a security policy), so it was written in place"
            ),
            leftover,
        ),
        Written::InPlace(InPlace::OwnerNotKept { leftover }) => (
            format!(
                "{path} belongs to another owner or group that a new file could not be given, \
                 so it was written in place to keep them"
            ),
            leftover,
        ),
        Written::InPlace(InPlace::DirectoryNotWritable) => (
            format!("the directory of {path} takes no new file, so it was written in place"),
            None,
        ),
    };
    let left = match leftover {
        Some(temp) => format!(
            "; its temporary file {} could not be removed: delete it",
            temp.display()
        ),
        None => String::new(),
    };
    Some(format!("[note: {why}, not atomically{left}]"))
}

/// Replace the contents of `path` whole (#2243), off the async runtime.
pub(super) async fn write_file(path: PathBuf, bytes: Vec<u8>) -> Result<Written, WriteFailure> {
    run_write(move || replace_contents(&path, &bytes)).await
}

/// Run a write off the async runtime. A write task that panicked is an
/// internal fault: what it left is not known, and is said so (#2254 review).
async fn run_write(
    write: impl FnOnce() -> Result<Written, WriteFailure> + Send + 'static,
) -> Result<Written, WriteFailure> {
    match tokio::task::spawn_blocking(write).await {
        Ok(written) => written,
        Err(error) => Err(WriteFailure {
            error: std::io::Error::other(format!("the write task failed: {error}")),
            file: FileState::Unknown,
            leftover: None,
        }),
    }
}

/// Leading bytes that mark a binary format: executables, archives and
/// compressed data.
const BINARY_MAGIC: &[&[u8]] = &[
    b"\x7fELF",            // ELF executable or library
    b"\xcf\xfa\xed\xfe",   // Mach-O, 64-bit
    b"\xce\xfa\xed\xfe",   // Mach-O, 32-bit
    b"\xca\xfe\xba\xbe",   // Mach-O universal, or Java class
    b"\0asm",              // WebAssembly
    b"PK\x03\x04",         // zip, jar, docx
    b"\x1f\x8b",           // gzip
    b"\xfd7zXZ\0",         // xz
    b"(\xb5/\xfd",         // zstd
    b"7z\xbc\xaf\x27\x1c", // 7-zip
    b"%PDF-",              // PDF
    b"SQLite format 3\0",  // SQLite database
];

/// Bytes that are clearly binary: a NUL byte, which text does not hold, or
/// a known binary format's leading bytes.
fn is_clearly_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0) || BINARY_MAGIC.iter().any(|magic| bytes.starts_with(magic))
}

/// `path` is not UTF-8: named as UTF-16 when it starts with a UTF-16
/// byte-order mark (convert it), as binary when it clearly is (inspect it),
/// else as binary or another encoding (inspect or convert it).
pub(super) fn not_utf8_text(access: Access, path: &str, bytes: &[u8]) -> String {
    let size = format_size(bytes.len());
    let hint = shell_escape_single(path);
    let edit_only = match access {
        Access::Edit => ", and edit changes UTF-8 text only",
        Access::Read | Access::List => "",
    };
    let is_utf16 = bytes.starts_with(b"\xff\xfe") || bytes.starts_with(b"\xfe\xff");
    if is_utf16 {
        return format!(
            "{path} is not UTF-8 text: it is UTF-16 text ({size}, it starts with a UTF-16 \
             byte-order mark){edit_only}. Convert it with bash, e.g. iconv -f UTF-16 -t UTF-8 {hint}"
        );
    }
    match is_clearly_binary(bytes) {
        true => format!(
            "{path} is not UTF-8 text: it is a binary file ({size}){edit_only}. Inspect it \
             with bash, e.g. file {hint} or xxd {hint} | head -n 40"
        ),
        false => format!(
            "{path} is not UTF-8 text ({size}): binary, or text in another encoding{edit_only}. \
             Inspect it with bash, e.g. xxd {hint} | head -n 40, or convert it, e.g. \
             iconv -f latin1 -t utf-8 {hint}"
        ),
    }
}

/// The most symbolic links the system follows in resolving one path
/// (Linux's `MAXSYMLINKS`); more is refused as a loop (`ELOOP`).
const SYMLOOP_MAX: usize = 40;

/// How a chain of symbolic links from a path the system refused with
/// `ELOOP` goes, walked by reading each link, never following one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkChain {
    /// A link repeats: following it leads back to itself.
    Cycle,
    /// More links than the system follows, none repeated.
    TooLong,
    /// The path is no link, or its chain ends early: the loop is in a
    /// directory on the way.
    Other,
}

/// Walk the links from `path` (at most `2 * SYMLOOP_MAX` of them), telling
/// a real cycle (a link seen again) from a chain that is only too long.
async fn walk_links(path: &Path) -> LinkChain {
    let mut seen = std::collections::HashSet::new();
    let mut link = plain_path(path).await;
    for hops in 0..=2 * SYMLOOP_MAX {
        if !seen.insert(link.clone()) {
            return LinkChain::Cycle;
        }
        let Ok(target) = tokio::fs::read_link(&link).await else {
            return match hops > SYMLOOP_MAX {
                true => LinkChain::TooLong,
                false => LinkChain::Other,
            };
        };
        let next = match link.parent() {
            Some(dir) if target.is_relative() => dir.join(target),
            Some(_) | None => target,
        };
        link = plain_path(&next).await;
    }
    LinkChain::TooLong
}

/// `path` with its directory resolved, so one link is one path however it
/// was spelled; as given when the directory cannot be resolved.
async fn plain_path(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => tokio::fs::canonicalize(dir)
            .await
            .map_or_else(|_| path.to_path_buf(), |dir| dir.join(name)),
        _ => path.to_path_buf(),
    }
}

/// For a path that was not found: the target that does not exist when the
/// path is a symbolic link, following a chain of links and resolving
/// relative targets against each link's own directory (#2166); `None` when
/// the path is not a link or its chain cannot be walked.
async fn dangling_link_target(path: &Path) -> Option<PathBuf> {
    let mut link = path.to_path_buf();
    // Bounded as the system bounds it: a longer chain, or a loop, is
    // refused by the system as a loop (`ELOOP`), not as missing.
    for _ in 0..SYMLOOP_MAX {
        let target = tokio::fs::read_link(&link).await.ok()?;
        let target = match link.parent() {
            Some(dir) if target.is_relative() => dir.join(target),
            Some(_) | None => target,
        };
        match tokio::fs::symlink_metadata(&target).await {
            Ok(meta) if meta.file_type().is_symlink() => link = target,
            Ok(_) => return None,
            Err(_) => {
                // Shown plainly: its directory exists, so resolve it.
                let plain = match (target.parent(), target.file_name()) {
                    (Some(dir), Some(name)) => tokio::fs::canonicalize(dir)
                        .await
                        .map_or_else(|_| target.clone(), |dir| dir.join(name)),
                    _ => target,
                };
                return Some(plain);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "fs_failure_tests.rs"]
mod tests;
