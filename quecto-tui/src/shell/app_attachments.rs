//! Images attached to the composed message (#2425): `/image <path>`,
//! `Ctrl+V`, the chips above the editor and the keys that edit them.
//!
//! The admission policy is `conversation::image_attachments` (which defers
//! every image rule to `quecto-image`); this file is the shell's half: it
//! reads the file or the clipboard off the event loop, and shows the chips
//! and the notices.
//!
//! Keys, while chips are pending: `Backspace` in an empty editor removes
//! the last chip, but a held Backspace that emptied the text stops there
//! (its key repeat leaves the chips alone); `Esc` on the master with no run
//! clears the chips with the editor text (a running agent's `Esc` still
//! aborts, a focused sub-agent's `Esc` still leaves it, and the chips stay);
//! `Ctrl+C` clears them with the editor before it would abort; `Enter` in
//! an empty editor sends the images alone. The chips belong to the
//! conversation: `/clear`, `/new` and a resume drop them, with a notice.

use super::*;
use crate::conversation::image_attachments::{
    AttachRefusal, PendingImages, clipboard_label, file_label, image_count_phrase, refusal_notice,
    resolve_image_path,
};
use crate::shell::clipboard_image::{ClipboardRead, ClipboardReader, NoClipboard};
use std::io::Read;
use std::sync::Arc;

/// A finished read of an attachment source, run off the event loop, and
/// the conversation it was started in.
pub(super) struct AttachmentRead {
    conversation: u64,
    source: ReadSource,
}

enum ReadSource {
    Clipboard(ClipboardRead),
    /// A `/image` file: the name its chip shows, and its bytes or why not.
    File {
        name: String,
        read: Result<Vec<u8>, String>,
    },
}

/// The composer's attachments and the sources they come from.
pub(super) struct AttachmentFlow {
    pub(super) pending: PendingImages,
    clipboard: Arc<dyn ClipboardReader>,
    /// A clipboard read is running; a second `Ctrl+V` is ignored meanwhile.
    clipboard_reading: bool,
    /// `/image` file reads running.
    file_reads: usize,
    /// When the last Backspace that met text (or a key repeat of one) came:
    /// a Backspace soon after it is that key held down, not a fresh press.
    text_backspace_at: Option<tokio::time::Instant>,
    /// Counts the conversations the composer has served: a read started in
    /// an earlier one attaches nothing to this one.
    conversation: u64,
    read_tx: mpsc::Sender<AttachmentRead>,
    pub(super) read_rx: mpsc::Receiver<AttachmentRead>,
}

impl AttachmentFlow {
    pub(super) fn new() -> Self {
        let (read_tx, read_rx) = mpsc::channel(READ_QUEUE);
        Self {
            pending: PendingImages::default(),
            clipboard: Arc::new(NoClipboard),
            clipboard_reading: false,
            file_reads: 0,
            text_backspace_at: None,
            conversation: 0,
            read_tx,
            read_rx,
        }
    }

    /// Whether a read has been started and not yet applied (the harness
    /// delivers reads the way the event loop's arm does).
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn reads_in_flight(&self) -> bool {
        self.clipboard_reading || self.file_reads > 0
    }
}

/// Room for the reads that can finish before the loop applies them.
const READ_QUEUE: usize = 16;

/// The most characters of a file or clipboard name kept for its chip.
const LABEL_CHARS: usize = 120;

/// A Backspace this soon after one that met text is that key held down:
/// longer than any common initial key-repeat delay (X11's default is
/// 660 ms), so a held Backspace stops at the start of the text.
const KEY_REPEAT_GAP: Duration = Duration::from_millis(700);

/// Sends a read's answer, or `fallback` if it is dropped without one (a
/// reader that panicked), so the read's in-flight latch always clears.
struct ReadAnswer {
    tx: mpsc::Sender<AttachmentRead>,
    fallback: Option<AttachmentRead>,
}

impl ReadAnswer {
    fn send(mut self, read: AttachmentRead) {
        self.fallback = None;
        let _ = self.tx.blocking_send(read);
    }
}

impl Drop for ReadAnswer {
    fn drop(&mut self) {
        if let Some(fallback) = self.fallback.take() {
            let _ = self.tx.blocking_send(fallback);
        }
    }
}

impl App {
    /// Read the clipboard through `reader` from now on; the CLI wires in the
    /// system clipboard, tests a fake.
    pub fn set_clipboard(&mut self, reader: Arc<dyn ClipboardReader>) {
        self.attachments.clipboard = reader;
    }

    /// `/image <path>`: read the image file `arg` names off the event loop;
    /// the result comes back through [`AttachmentFlow::read_rx`].
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
        self.attachments.file_reads += 1;
        let conversation = self.attachments.conversation;
        let answer = ReadAnswer {
            tx: self.attachments.read_tx.clone(),
            fallback: Some(AttachmentRead {
                conversation,
                source: ReadSource::File {
                    name: name.clone(),
                    read: Err("the read stopped".to_string()),
                },
            }),
        };
        tokio::task::spawn_blocking(move || {
            let read = read_image_file(&path).map_err(|error| error.to_string());
            let source = ReadSource::File { name, read };
            answer.send(AttachmentRead {
                conversation,
                source,
            });
        });
    }

    /// Admit `bytes` as the image `name`, read in `conversation`, or show
    /// why not.
    fn attach_image_bytes(&mut self, name: &str, bytes: &[u8], conversation: u64) {
        let name = sanitize_label(name);
        let admitted = match conversation == self.attachments.conversation {
            true => self.attachments.pending.admit(&name, bytes),
            false => {
                let notice = format!("Image not attached: {name}: the conversation changed");
                self.notify(&notice, NotifyLevel::Warning);
                return;
            }
        };
        match admitted {
            Ok(()) => {}
            Err(refusal) => self.notify(&refusal_notice(&name, &refusal), NotifyLevel::Error),
        }
    }

    /// `Ctrl+V`: read the clipboard off the event loop; the result comes back
    /// through [`AttachmentFlow::read_rx`]. A press while a read runs is
    /// ignored, and says so.
    pub(super) fn start_clipboard_read(&mut self) {
        match self.attachments.clipboard_reading {
            true => self.notify("Already reading the clipboard", NotifyLevel::Info),
            false => {
                self.attachments.clipboard_reading = true;
                let reader = Arc::clone(&self.attachments.clipboard);
                let conversation = self.attachments.conversation;
                let answer = ReadAnswer {
                    tx: self.attachments.read_tx.clone(),
                    fallback: Some(AttachmentRead {
                        conversation,
                        source: ReadSource::Clipboard(ClipboardRead::Failed(
                            "the clipboard reader stopped".to_string(),
                        )),
                    }),
                };
                tokio::task::spawn_blocking(move || {
                    let source = ReadSource::Clipboard(reader.read());
                    answer.send(AttachmentRead {
                        conversation,
                        source,
                    });
                });
            }
        }
    }

    /// Act on a finished attachment read.
    pub(super) fn apply_attachment_read(&mut self, read: AttachmentRead) {
        let conversation = read.conversation;
        match read.source {
            ReadSource::Clipboard(read) => {
                self.attachments.clipboard_reading = false;
                self.apply_clipboard_read(read, conversation);
            }
            ReadSource::File { name, read } => {
                assert!(self.attachments.file_reads > 0, "a file read was started");
                self.attachments.file_reads -= 1;
                match read {
                    Ok(bytes) => self.attach_image_bytes(&name, &bytes, conversation),
                    Err(error) => {
                        let notice =
                            format!("Image not attached: {}: {error}", sanitize_label(&name));
                        self.notify(&notice, NotifyLevel::Error);
                    }
                }
            }
        }
    }

    /// Act on a finished clipboard read: attach an image, paste text, or say
    /// in one line why nothing was attached.
    fn apply_clipboard_read(&mut self, read: ClipboardRead, conversation: u64) {
        match read {
            ClipboardRead::Image(bytes) => {
                self.attach_image_bytes(&clipboard_label(&bytes), &bytes, conversation)
            }
            ClipboardRead::Text(text) => self.handle_key(Key::Paste(text)),
            ClipboardRead::UnsupportedImage(types) => {
                let name = format!("clipboard ({})", sanitize_label(&types));
                self.notify(
                    &refusal_notice(&name, &AttachRefusal::NotAnImage),
                    NotifyLevel::Error,
                );
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

    /// Drop the chips when their conversation ends (`/clear`, `/new`, a
    /// resume), saying how many were not sent.
    pub(super) fn drop_pending_images(&mut self) {
        self.attachments.conversation = self.attachments.conversation.wrapping_add(1);
        match self.attachments.pending.len() {
            0 => {}
            count => {
                self.attachments.pending.clear();
                let notice = format!(
                    "{} not sent: the conversation changed",
                    image_count_phrase(count)
                );
                self.notify(&notice, NotifyLevel::Warning);
            }
        }
    }

    /// The keys that edit the attachments; whether `key` was one of them.
    pub(super) fn handle_attachment_key(&mut self, key: &Key) -> bool {
        let now = self.clock.now();
        let held_backspace = matches!(key, Key::Backspace)
            && self
                .attachments
                .text_backspace_at
                .is_some_and(|at| now.saturating_duration_since(at) < KEY_REPEAT_GAP);
        let chips = !self.attachments.pending.is_empty();
        let editor_blank = self.editor.text().trim().is_empty();
        let editor_empty = self.editor.text().is_empty();
        let master_idle =
            self.ac().roster.active_agent_id.is_none() && !self.ac().agent_state.is_running();
        // Only a Backspace that meets text, or the key repeat of one, keeps
        // the run going.
        self.attachments.text_backspace_at = match (key, editor_empty, held_backspace) {
            (Key::Backspace, false, _) | (Key::Backspace, true, true) => Some(now),
            _ => None,
        };
        match (key, chips) {
            (Key::Ctrl('v'), _) => self.start_clipboard_read(),
            (Key::Backspace, true) if editor_empty && held_backspace => {}
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
