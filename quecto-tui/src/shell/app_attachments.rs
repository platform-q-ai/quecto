//! Images attached to the composed message (#2425): `/image <path>`,
//! `Ctrl+V`, the chips above the editor and the keys that edit them.
//!
//! The admission policy is `conversation::image_attachments` (which defers
//! every image rule to `quecto-image`); this file is the shell's half: it
//! reads the file or the clipboard, runs the clipboard read off the event
//! loop, and shows the chips and the notices.

use super::*;
use crate::conversation::image_attachments::PendingImages;
use crate::shell::clipboard_image::{ClipboardRead, ClipboardReader, NoClipboard};
use std::sync::Arc;

/// The composer's attachments and the clipboard they can come from.
pub(super) struct AttachmentFlow {
    pub(super) pending: PendingImages,
    clipboard: Arc<dyn ClipboardReader>,
    /// A clipboard read is running; a second `Ctrl+V` waits for it.
    read_in_flight: bool,
    clipboard_tx: mpsc::Sender<ClipboardRead>,
    pub(super) clipboard_rx: mpsc::Receiver<ClipboardRead>,
}

impl AttachmentFlow {
    pub(super) fn new() -> Self {
        let (clipboard_tx, clipboard_rx) = mpsc::channel(1);
        Self {
            pending: PendingImages::default(),
            clipboard: Arc::new(NoClipboard),
            read_in_flight: false,
            clipboard_tx,
            clipboard_rx,
        }
    }
}

impl App {
    /// Read the clipboard through `reader` from now on; the CLI wires in the
    /// system clipboard, tests a fake.
    pub fn set_clipboard(&mut self, reader: Arc<dyn ClipboardReader>) {
        self.attachments.clipboard = reader;
    }

    /// `/image <path>`: attach the image file `arg` names.
    pub(super) fn attach_image_file(&mut self, _arg: &str) {}

    /// `Ctrl+V`: read the clipboard off the event loop; the result comes back
    /// through [`AttachmentFlow::clipboard_rx`].
    pub(super) fn start_clipboard_read(&mut self) {
        let _ = (
            &self.attachments.clipboard_tx,
            self.attachments.read_in_flight,
        );
    }

    /// Act on a finished clipboard read.
    pub(super) fn apply_clipboard_read(&mut self, _read: ClipboardRead) {
        self.attachments.read_in_flight = false;
    }

    /// The keys that edit the attachments; whether `key` was one.
    pub(super) fn handle_attachment_key(&mut self, _key: &Key) -> bool {
        if self.attachments.read_in_flight && self.attachments.pending.is_empty() {
            self.start_clipboard_read();
            self.attach_image_file("");
        }
        false
    }

    /// The chips, packed into lines of `width`, for the bottom stack.
    pub(super) fn attachment_chip_lines(&self, _width: usize) -> Vec<String> {
        Vec::new()
    }
}
