//! Reading the system clipboard for `Ctrl+V` (#2425): an image to attach,
//! or text to paste.
//!
//! [`ClipboardReader`] is the port the App reads through; tests hand it a
//! fake, so no test touches the real clipboard. [`SystemClipboard`] is the
//! adapter the CLI wires in: it runs one of an allowlist of two tools,
//! `wl-paste` (Wayland) and `xclip` (X11), each command under a timeout,
//! and never anything else.

use std::path::PathBuf;
use std::time::Duration;

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
    pub fn for_session(_wayland: bool, _x11: bool) -> Vec<Self> {
        Vec::new()
    }
}

/// The system clipboard, read through the first tool of `tools` that runs.
pub struct SystemClipboard {
    tools: Vec<(ClipboardTool, PathBuf)>,
    timeout: Duration,
}

impl SystemClipboard {
    /// The tools this session's environment names (`WAYLAND_DISPLAY`,
    /// `DISPLAY`), each found on `PATH`.
    pub fn from_env() -> Self {
        Self::with_tools(Vec::new(), CLIPBOARD_TIMEOUT)
    }

    /// `tools` run from the given program paths, each command limited to
    /// `timeout`.
    pub fn with_tools(tools: Vec<(ClipboardTool, PathBuf)>, timeout: Duration) -> Self {
        Self { tools, timeout }
    }
}

impl ClipboardReader for SystemClipboard {
    fn read(&self) -> ClipboardRead {
        let _ = (&self.tools, self.timeout);
        ClipboardRead::NoTool
    }
}

#[cfg(test)]
#[path = "clipboard_image_tests.rs"]
mod tests;
