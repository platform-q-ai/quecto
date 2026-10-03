//! Images attached to the composed message (#2425): `/image <path>`,
//! `Ctrl+V`, the chips above the editor and the keys that edit them.
//!
//! The admission policy is `conversation::image_attachments` (which defers
//! every image rule to `quecto-image`); this file is the shell's half: it
//! reads the file or the clipboard off the event loop, and shows the chips
//! and the notices.
//!
//! Keys, while chips are pending: `Backspace` in an empty editor removes
//! the last chip, but a held Backspace stops at the start of the text and
//! removes at most one chip (its key repeats leave the chips alone, with a
//! one-time hint); `Esc` on the master with no run
//! clears the chips with the editor text (a running agent's `Esc` still
//! aborts, a focused sub-agent's `Esc` still leaves it, and the chips stay);
//! `Ctrl+C` clears them with the editor before it would abort; `Enter` in
//! an empty editor sends the images alone. The chips belong to the
//! conversation: `/clear`, `/new` and a resume drop them, with a notice.
//! A message is held back while an image is still being read.

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
    /// When the last Backspace that did something (met text, removed a
    /// chip) or was a key repeat of one came: a Backspace soon after it is
    /// that key held down, not a fresh press. Only an editing key ends it.
    backspace_held_at: Option<tokio::time::Instant>,
    /// The "held Backspace" hint has been shown (it is shown once).
    held_hint_shown: bool,
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
            backspace_held_at: None,
            held_hint_shown: false,
            conversation: 0,
            read_tx,
            read_rx,
        }
    }

    /// Whether a read has been started and not yet applied: a message sent
    /// meanwhile would leave its image behind.
    pub(super) fn reads_in_flight(&self) -> bool {
        self.clipboard_reading || self.file_reads > 0
    }
}

/// Room for the reads that can finish before the loop applies them.
const READ_QUEUE: usize = 16;

/// The most characters of a file or clipboard name kept for its chip.
const LABEL_CHARS: usize = 120;

/// The hint for a Backspace that would have removed a chip but was taken
/// for the key repeat of a held one (shown once).
const HELD_BACKSPACE_HINT: &str = "Wait a moment, then press Backspace again to remove the image";

/// A Backspace this soon after one that did something is that key held
/// down: longer than the common initial key-repeat delays (X11's default is
/// 660 ms), so a held Backspace stops at the start of the text and removes
/// at most one chip. No terminal tells a repeat from a fresh press, so a
/// slower first repeat counts as a press; the fast repeats after it do not.
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
        let chips = !self.attachments.pending.is_empty();
        let editor_blank = self.editor.text().trim().is_empty();
        let editor_empty = self.editor.text().is_empty();
        let held_backspace = matches!(key, Key::Backspace)
            && self
                .attachments
                .backspace_held_at
                .is_some_and(|at| now.saturating_duration_since(at) < KEY_REPEAT_GAP);
        let master_idle =
            self.ac().roster.active_agent_id.is_none() && !self.ac().agent_state.is_running();
        // A Backspace that meets text, removes a chip or repeats one keeps
        // the hold going; an editing key ends it; anything else (the wheel,
        // the mouse) leaves it be.
        self.attachments.backspace_held_at = match (key, editor_empty, chips || held_backspace) {
            (Key::Backspace, false, _) | (Key::Backspace, true, true) => Some(now),
            (Key::Backspace, true, false) => None,
            (key, _, _) if is_editing_key(key) => None,
            _ => self.attachments.backspace_held_at,
        };
        match (key, chips) {
            (Key::Ctrl('v'), _) => self.start_clipboard_read(),
            (Key::Backspace, true) if editor_empty && held_backspace => {
                match self.attachments.held_hint_shown {
                    true => {}
                    false => {
                        self.attachments.held_hint_shown = true;
                        self.notify(HELD_BACKSPACE_HINT, NotifyLevel::Info);
                    }
                }
            }
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

/// Whether `key` edits or moves in the editor (an allowlist): such a key
/// ends a held Backspace. The wheel, the mouse and paging do not.
fn is_editing_key(key: &Key) -> bool {
    matches!(
        key,
        Key::Char(_)
            | Key::Enter
            | Key::ShiftEnter
            | Key::Escape
            | Key::Delete
            | Key::Insert
            | Key::Tab
            | Key::BackTab
            | Key::Paste(_)
            | Key::Alt(_)
            | Key::Ctrl(_)
            | Key::CtrlShift(_)
            | Key::Left
            | Key::Right
            | Key::Up
            | Key::Down
            | Key::CtrlLeft
            | Key::CtrlRight
            | Key::Home
            | Key::End
    )
}

/// A file or clipboard name made safe to show: control characters dropped,
/// bounded in length.
fn sanitize_label(name: &str) -> String {
    crate::components::ansi::sanitize_untrusted_label(name, LABEL_CHARS)
}

/// Read the regular file `path`, at most one byte past the image limit: the
/// admission check refuses an over-limit image, so the rest is never read.
/// Symlinks are resolved first; the resolved file is then opened without
/// following a link and without blocking, and only a regular file (by
/// `fstat` of what was opened, so nothing swapped in after a check) is read:
/// a directory or a FIFO is refused, never waited on.
fn read_image_file(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let resolved = std::fs::canonicalize(path)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&resolved)?;
    match file.metadata()?.is_file() {
        true => {}
        false => return Err(std::io::Error::other("not a regular file")),
    }
    let mut bytes = Vec::new();
    file.take(quecto_image::MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
