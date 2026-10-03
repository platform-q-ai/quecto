//! Images attached to the composed message (#2425): `/image <path>`,
//! `Ctrl+V`, the chips above the editor and the keys that edit them.
//!
//! The admission policy is `conversation::image_attachments` (which defers
//! every image rule to `quecto-image`); this file is the shell's half: it
//! reads the file or the clipboard, runs the clipboard read off the event
//! loop, and shows the chips and the notices.
//!
//! Keys, while chips are pending: `Backspace` in an empty editor removes
//! the last chip; `Esc` with no run to abort clears the chips with the
//! editor text (a running agent's `Esc` still aborts, and the chips stay);
//! `Ctrl+C` clears them with the editor before it would abort; `Enter` in
//! an empty editor sends the images alone.

use super::*;
use crate::conversation::image_attachments::{
    PendingImages, clipboard_label, file_label, refusal_notice, resolve_image_path,
};
use crate::shell::clipboard_image::{ClipboardRead, ClipboardReader, NoClipboard};
use std::io::Read;
use std::sync::Arc;

/// A finished read of an attachment source, run off the event loop.
pub(super) enum AttachmentRead {
    Clipboard(ClipboardRead),
}

/// The composer's attachments and the clipboard they can come from.
pub(super) struct AttachmentFlow {
    pub(super) pending: PendingImages,
    clipboard: Arc<dyn ClipboardReader>,
    /// A clipboard read is running; a second `Ctrl+V` waits for it.
    read_in_flight: bool,
    read_tx: mpsc::Sender<AttachmentRead>,
    pub(super) read_rx: mpsc::Receiver<AttachmentRead>,
}

impl AttachmentFlow {
    pub(super) fn new() -> Self {
        let (read_tx, read_rx) = mpsc::channel(1);
        Self {
            pending: PendingImages::default(),
            clipboard: Arc::new(NoClipboard),
            read_in_flight: false,
            read_tx,
            read_rx,
        }
    }

    /// Whether a read has been started and not yet applied.
    pub(super) fn reads_in_flight(&self) -> bool {
        self.read_in_flight
    }
}

/// The most characters of a file or clipboard name kept for its chip.
const LABEL_CHARS: usize = 120;

impl App {
    /// Read the clipboard through `reader` from now on; the CLI wires in the
    /// system clipboard, tests a fake.
    pub fn set_clipboard(&mut self, reader: Arc<dyn ClipboardReader>) {
        self.attachments.clipboard = reader;
    }

    /// `/image <path>`: attach the image file `arg` names.
    pub(super) fn attach_image_file(&mut self, arg: &str) {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let workspace = self.files_autocomplete_root();
        let path = match resolve_image_path(arg, home.as_deref(), &workspace) {
            Ok(path) => path,
            Err(error) => {
                self.notify(&error.to_string(), NotifyLevel::Warning);
                return;
            }
        };
        let name = file_label(&path);
        match read_image_file(&path) {
            Ok(bytes) => self.attach_image_bytes(&name, &bytes),
            Err(error) => {
                let notice = format!("Image not attached: {}: {error}", sanitize_label(&name));
                self.notify(&notice, NotifyLevel::Error);
            }
        }
    }

    /// Admit `bytes` as the image `name`, or show why not.
    fn attach_image_bytes(&mut self, name: &str, bytes: &[u8]) {
        let name = sanitize_label(name);
        match self.attachments.pending.admit(&name, bytes) {
            Ok(()) => {}
            Err(refusal) => self.notify(&refusal_notice(&name, &refusal), NotifyLevel::Error),
        }
    }

    /// `Ctrl+V`: read the clipboard off the event loop; the result comes back
    /// through [`AttachmentFlow::read_rx`]. A press while a read runs
    /// waits for that read.
    pub(super) fn start_clipboard_read(&mut self) {
        match self.attachments.read_in_flight {
            true => {}
            false => {
                self.attachments.read_in_flight = true;
                let reader = Arc::clone(&self.attachments.clipboard);
                let tx = self.attachments.read_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let _ = tx.blocking_send(AttachmentRead::Clipboard(reader.read()));
                });
            }
        }
    }

    /// Act on a finished attachment read.
    pub(super) fn apply_attachment_read(&mut self, read: AttachmentRead) {
        match read {
            AttachmentRead::Clipboard(read) => self.apply_clipboard_read(read),
        }
    }

    /// Act on a finished clipboard read: attach an image, paste text, or say
    /// in one line why nothing was attached.
    fn apply_clipboard_read(&mut self, read: ClipboardRead) {
        self.attachments.read_in_flight = false;
        match read {
            ClipboardRead::Image(bytes) => {
                self.attach_image_bytes(&clipboard_label(&bytes), &bytes)
            }
            ClipboardRead::Text(text) => self.handle_key(Key::Paste(text)),
            ClipboardRead::UnsupportedImage(types) => {
                let admitted: Vec<&str> = quecto_image::ImageMime::ALL
                    .iter()
                    .map(|mime| mime.as_str())
                    .collect();
                let notice = format!(
                    "Clipboard image not attached: {} is not one of {}",
                    sanitize_label(&types),
                    admitted.join(", ")
                );
                self.notify(&notice, NotifyLevel::Error);
            }
            ClipboardRead::NotPasteable(types) => self.notify(
                &format!(
                    "Nothing to paste: the clipboard holds only {}, neither an image nor text",
                    sanitize_label(&types)
                ),
                NotifyLevel::Info,
            ),
            ClipboardRead::Empty => self.notify(
                "Nothing to paste: the clipboard is empty",
                NotifyLevel::Info,
            ),
            ClipboardRead::NoTool => self.notify(
                "Image paste needs wl-paste (Wayland) or xclip (X11); paste text with the terminal",
                NotifyLevel::Warning,
            ),
            ClipboardRead::Failed(reason) => self.notify(
                &format!("Clipboard read failed: {}", sanitize_label(&reason)),
                NotifyLevel::Error,
            ),
        }
    }

    /// The keys that edit the attachments; whether `key` was one of them.
    pub(super) fn handle_attachment_key(&mut self, key: &Key) -> bool {
        let chips = !self.attachments.pending.is_empty();
        let editor_blank = self.editor.text().trim().is_empty();
        let editor_empty = self.editor.text().is_empty();
        let master_idle =
            self.ac().roster.active_agent_id.is_none() && !self.ac().agent_state.is_running();
        match (key, chips) {
            (Key::Ctrl('v'), _) => self.start_clipboard_read(),
            (Key::Backspace, true) if editor_empty => {
                self.attachments.pending.remove_last();
            }
            (Key::Escape, true) if master_idle => {
                self.ac_mut().rewind.last_idle_escape = None;
                self.attachments.pending.clear();
                self.editor.set_text("");
                self.autocomplete.dismiss();
            }
            (Key::Enter, true) if editor_blank => {
                self.editor.set_text("");
                self.handle_submit("");
            }
            _ => return false,
        }
        true
    }

    /// The chips, packed greedily into lines of `width`, for the bottom
    /// stack; a chip wider than `width` is cut.
    pub(super) fn attachment_chip_lines(&self, width: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for chip in self.attachments.pending.chips() {
            let joined = lines.last().map(|line| format!("{line}  {chip}"));
            match joined {
                Some(joined) if crate::components::utils::visible_width(&joined) <= width => {
                    *lines.last_mut().expect("a line to join") = joined;
                }
                _ => lines.push(chip),
            }
        }
        lines
            .iter()
            .map(|line| {
                let line = crate::components::utils::truncate_to_width(line, width, Some("…"));
                crate::components::theme::accent(&line)
            })
            .collect()
    }
}

/// A file or clipboard name made safe to show: control characters dropped,
/// bounded in length.
fn sanitize_label(name: &str) -> String {
    crate::components::ansi::sanitize_untrusted_label(name, LABEL_CHARS)
}

/// Read the regular file `path`, at most one byte past the image limit: the
/// admission check refuses an over-limit image, so the rest is never read.
/// Anything but a regular file (a directory, a FIFO that would block) is
/// refused before it is opened.
fn read_image_file(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    match std::fs::metadata(path)?.is_file() {
        true => {}
        false => return Err(std::io::Error::other("not a regular file")),
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(quecto_image::MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
