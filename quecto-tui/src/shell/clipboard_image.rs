//! Reading the system clipboard for `Ctrl+V` (#2425): an image to attach,
//! or text to paste.
//!
//! [`ClipboardReader`] is the port the App reads through; tests hand it a
//! fake, so no test touches the real clipboard. [`SystemClipboard`] is the
//! adapter the CLI wires in: it runs one of an allowlist of two tools,
//! `wl-paste` (Wayland) and `xclip` (X11), each command under a timeout,
//! and never anything else.

use quecto_image::ImageMime;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long one clipboard command may run before it is killed.
pub const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(2);

/// What one read of the clipboard found.
#[derive(Clone, PartialEq, Eq)]
pub enum ClipboardRead {
    /// An image of an admitted type: its bytes, as the tool gave them.
    Image(Vec<u8>),
    /// No image, but text: paste it as typed text.
    Text(String),
    /// An image of a type quecto does not admit, and no text; holds the
    /// types offered.
    UnsupportedImage(String),
    /// Neither an image nor text: holds the types offered (e.g.
    /// `text/uri-list` from a file manager).
    NotPasteable(String),
    /// Nothing to paste.
    Empty,
    /// Neither clipboard tool is available.
    NoTool,
    /// A clipboard command failed; holds why.
    Failed(String),
}

/// Never prints an image's bytes.
impl std::fmt::Debug for ClipboardRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(bytes) => write!(f, "Image({} bytes)", bytes.len()),
            Self::Text(text) => write!(f, "Text({} chars)", text.chars().count()),
            Self::UnsupportedImage(types) => write!(f, "UnsupportedImage({types:?})"),
            Self::NotPasteable(types) => write!(f, "NotPasteable({types:?})"),
            Self::Empty => f.write_str("Empty"),
            Self::NoTool => f.write_str("NoTool"),
            Self::Failed(reason) => write!(f, "Failed({reason:?})"),
        }
    }
}

/// The port: one blocking read of the clipboard. The App runs it off the
/// event loop.
pub trait ClipboardReader: Send + Sync {
    fn read(&self) -> ClipboardRead;
}

/// The App's reader until the CLI wires in [`SystemClipboard`]: there is no
/// clipboard tool.
pub struct NoClipboard;

impl ClipboardReader for NoClipboard {
    fn read(&self) -> ClipboardRead {
        ClipboardRead::NoTool
    }
}

/// A clipboard tool quecto runs, and the only two it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardTool {
    WlPaste,
    Xclip,
}

impl ClipboardTool {
    /// The program's name, found on `PATH`.
    pub fn program(self) -> &'static str {
        match self {
            Self::WlPaste => "wl-paste",
            Self::Xclip => "xclip",
        }
    }

    /// The tools a session can use, in the order they are tried: `wl-paste`
    /// in a Wayland session, `xclip` in an X11 one (XWayland included).
    pub fn for_session(wayland: bool, x11: bool) -> Vec<Self> {
        [(Self::WlPaste, wayland), (Self::Xclip, x11)]
            .into_iter()
            .filter_map(|(tool, present)| present.then_some(tool))
            .collect()
    }

    /// The arguments that list the types the clipboard offers.
    fn list_args(self) -> Vec<String> {
        match self {
            Self::WlPaste => args(&["--list-types"]),
            Self::Xclip => args(&["-selection", "clipboard", "-t", "TARGETS", "-o"]),
        }
    }

    /// The arguments that read the clipboard as `mime`.
    fn image_args(self, mime: ImageMime) -> Vec<String> {
        let wanted = mime.as_str();
        match self {
            Self::WlPaste => args(&["--type", wanted]),
            Self::Xclip => args(&["-selection", "clipboard", "-t", wanted, "-o"]),
        }
    }

    /// Whether `stderr` from a listing that exited non-zero says the
    /// clipboard is empty, as each tool words it; any other failure is the
    /// tool failing to run (no display server, a stale `WAYLAND_DISPLAY`).
    fn says_empty(self, stderr: &str) -> bool {
        let empty: &[&str] = match self {
            Self::WlPaste => &["Nothing is copied", "No selection"],
            Self::Xclip => &["target TARGETS not available"],
        };
        empty.iter().any(|words| stderr.contains(words))
    }

    /// The arguments that read the clipboard as text.
    fn text_args(self) -> Vec<String> {
        match self {
            Self::WlPaste => args(&["--no-newline"]),
            Self::Xclip => args(&["-selection", "clipboard", "-o"]),
        }
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| arg.to_string()).collect()
}

/// The clipboard types that are text, as `wl-paste --list-types` and
/// `xclip -t TARGETS` name them.
const TEXT_TYPES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

/// The most text one paste takes: 1 MiB.
const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// The most of a tool's stderr kept to say why it failed.
const MAX_STDERR_BYTES: u64 = 4096;

/// The most a type listing may be.
const MAX_LISTING_BYTES: usize = 64 * 1024;

/// The system clipboard, read through the first tool of `tools` that runs.
pub struct SystemClipboard {
    tools: Vec<(ClipboardTool, PathBuf)>,
    timeout: Duration,
}

impl SystemClipboard {
    /// The tools this session's environment names (`WAYLAND_DISPLAY`,
    /// `DISPLAY`), each found on `PATH`.
    pub fn from_env() -> Self {
        let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        let tools = ClipboardTool::for_session(set("WAYLAND_DISPLAY"), set("DISPLAY"))
            .into_iter()
            .map(|tool| (tool, PathBuf::from(tool.program())))
            .collect();
        Self::with_tools(tools, CLIPBOARD_TIMEOUT)
    }

    /// `tools` run from the given program paths, each command limited to
    /// `timeout`.
    pub fn with_tools(tools: Vec<(ClipboardTool, PathBuf)>, timeout: Duration) -> Self {
        Self { tools, timeout }
    }

    /// Read what the listing `types` offers through `tool`: an admitted
    /// image first (in [`ImageMime::ALL`]'s order), else text.
    fn read_offered(&self, tool: ClipboardTool, program: &Path, types: &[&str]) -> ClipboardRead {
        let image = ImageMime::ALL
            .into_iter()
            .find(|mime| types.contains(&mime.as_str()));
        let text = types.iter().any(|kind| {
            TEXT_TYPES
                .iter()
                .any(|text| text.eq_ignore_ascii_case(kind))
        });
        // What the clipboard holds, as MIME types (`type/subtype`): X11's
        // own bookkeeping targets (`TARGETS`, `TIMESTAMP`, …) name nothing.
        let mime_types: Vec<&str> = types
            .iter()
            .copied()
            .filter(|kind| kind.contains('/'))
            .collect();
        let images_offered: Vec<&str> = mime_types
            .iter()
            .copied()
            .filter(|kind| kind.starts_with("image/"))
            .collect();
        match (image, text, images_offered.as_slice()) {
            (Some(mime), _, _) => {
                let read = run(
                    program,
                    &tool.image_args(mime),
                    self.timeout,
                    quecto_image::MAX_IMAGE_BYTES,
                );
                match read {
                    Ok(bytes) => ClipboardRead::Image(bytes),
                    Err(error) => {
                        ClipboardRead::Failed(format!("{} {mime}: {error}", tool.program()))
                    }
                }
            }
            (None, true, _) => {
                match run(program, &tool.text_args(), self.timeout, MAX_TEXT_BYTES) {
                    Ok(bytes) if bytes.len() <= MAX_TEXT_BYTES => {
                        ClipboardRead::Text(String::from_utf8_lossy(&bytes).into_owned())
                    }
                    Ok(_) => ClipboardRead::Failed(format!(
                        "the clipboard text is over {} MiB",
                        MAX_TEXT_BYTES / (1024 * 1024)
                    )),
                    Err(error) => {
                        ClipboardRead::Failed(format!("{} text: {error}", tool.program()))
                    }
                }
            }
            (None, false, []) => match mime_types.as_slice() {
                [] => ClipboardRead::Empty,
                held => ClipboardRead::NotPasteable(held.join(", ")),
            },
            (None, false, offered) => ClipboardRead::UnsupportedImage(offered.join(", ")),
        }
    }
}

impl ClipboardReader for SystemClipboard {
    fn read(&self) -> ClipboardRead {
        // Of the tools that ran and listed nothing: whether one said the
        // clipboard is empty, and why the others failed.
        let mut said_empty = false;
        let mut failures: Vec<String> = Vec::new();
        for (tool, program) in &self.tools {
            match run(program, &tool.list_args(), self.timeout, MAX_LISTING_BYTES) {
                Ok(listing) => {
                    let listing = String::from_utf8_lossy(&listing);
                    let types: Vec<&str> = listing.lines().map(str::trim).collect();
                    return self.read_offered(*tool, program, &types);
                }
                // Not installed: the next tool of the allowlist.
                Err(RunError::NotFound) => continue,
                // A listing exits non-zero when the clipboard holds nothing
                // (its stderr says so) and when the tool reaches no display
                // server (a stale `WAYLAND_DISPLAY`): either way the next
                // tool may still read it.
                Err(RunError::Exit(_, stderr)) if tool.says_empty(&stderr) => said_empty = true,
                Err(error) => failures.push(format!("{}: {error}", tool.program())),
            }
        }
        match (said_empty, failures.as_slice()) {
            (true, _) => ClipboardRead::Empty,
            (false, []) => ClipboardRead::NoTool,
            (false, failed) => ClipboardRead::Failed(failed.join("; ")),
        }
    }
}

/// Why a clipboard command gave no output.
#[derive(Debug)]
enum RunError {
    /// The program is not there.
    NotFound,
    /// It could not be started, or its output not read.
    Io(std::io::Error),
    /// It ran past the timeout and was killed.
    TimedOut(Duration),
    /// It exited unsuccessfully: its status and what it said on stderr.
    Exit(std::process::ExitStatus, String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Io(error) => write!(f, "{error}"),
            Self::TimedOut(after) => write!(f, "timed out after {} ms", after.as_millis()),
            Self::Exit(status, stderr) => match stderr.trim() {
                "" => write!(f, "{status}"),
                said => write!(f, "{said} ({status})"),
            },
        }
    }
}

/// Run `program args` with no stdin and stderr discarded, and return its
/// stdout: at most `cap + 1` bytes (one over says "too much"; the child is
/// then killed), within `timeout` (the child is killed and reaped past it).
fn run(
    program: &Path,
    args: &[String],
    timeout: Duration,
    cap: usize,
) -> Result<Vec<u8>, RunError> {
    let deadline = Instant::now() + timeout;
    // Its own process group, so a timeout ends whatever it forked too
    // (`wl-paste` hands the transfer to a child).
    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => RunError::NotFound,
            _ => RunError::Io(error),
        })?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
    // stderr says why a listing failed: keep its start, drain the rest so
    // the tool never blocks writing it.
    std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut said = Vec::new();
        let _ = (&mut stderr).take(MAX_STDERR_BYTES).read_to_end(&mut said);
        let _ = stderr_tx.send(said);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
    });
    let (tx, rx) = std::sync::mpsc::channel();
    // A thread reads, so the wait below can give up at the deadline.
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = stdout.take(cap as u64 + 1).read_to_end(&mut bytes);
        let _ = tx.send(read.map(|_| bytes));
    });
    let output = match rx.recv_timeout(timeout) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            end(&mut child);
            return Err(RunError::Io(error));
        }
        Err(_) => {
            end(&mut child);
            return Err(RunError::TimedOut(timeout));
        }
    };
    if output.len() > cap {
        // Over the cap: the rest is not wanted, and the child may block
        // writing it.
        end(&mut child);
        return Ok(output);
    }
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(output),
            Ok(Some(status)) => {
                let wait = deadline.saturating_duration_since(Instant::now());
                let said = stderr_rx.recv_timeout(wait).unwrap_or_default();
                return Err(RunError::Exit(
                    status,
                    String::from_utf8_lossy(&said).into(),
                ));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                end(&mut child);
                return Err(RunError::TimedOut(timeout));
            }
            Err(error) => {
                end(&mut child);
                return Err(RunError::Io(error));
            }
        }
    }
}

/// Kill `child`'s whole process group (SIGKILL), then reap `child`.
fn end(child: &mut std::process::Child) {
    let group = libc::pid_t::try_from(child.id()).expect("a pid fits pid_t");
    assert!(group > 0, "a child's process group is never 0 or negative");
    // SAFETY: killpg only sends a signal; `child` leads its own group (`process_group(0)`) and is unreaped, so its id names no other group.
    unsafe { libc::killpg(group, libc::SIGKILL) };
    let _ = child.wait();
}

#[cfg(test)]
#[path = "clipboard_image_tests.rs"]
mod tests;
