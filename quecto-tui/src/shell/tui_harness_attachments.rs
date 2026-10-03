//! Harness drivers for image attachments (#2425): a fake clipboard, the
//! `Ctrl+V` round trip the event loop runs, and the chips.

use super::TuiHarness;
use crate::shell::clipboard_image::{ClipboardRead, ClipboardReader};
use crate::shell::keys::Key;
use std::sync::Arc;

/// A clipboard that always holds `0`: the only clipboard tests read.
pub struct FakeClipboard(pub ClipboardRead);

impl ClipboardReader for FakeClipboard {
    fn read(&self) -> ClipboardRead {
        self.0.clone()
    }
}

impl TuiHarness {
    /// From now on the clipboard holds `read`.
    pub fn set_clipboard(&mut self, read: ClipboardRead) -> &mut Self {
        self.app.set_clipboard(Arc::new(FakeClipboard(read)));
        self
    }

    /// Press `Ctrl+V`, then deliver the clipboard read the way the event
    /// loop's arm does, and capture the frame.
    pub async fn paste_clipboard(&mut self) -> &mut Self {
        self.app.handle_key(Key::Ctrl('v'));
        self.settle_attachment_reads().await
    }

    /// Deliver every attachment read in flight (clipboard and `/image`
    /// files) the way the event loop's arm does, and capture the frame.
    pub async fn settle_attachment_reads(&mut self) -> &mut Self {
        while self.app.attachments.reads_in_flight() {
            let read = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                self.app.attachments.read_rx.recv(),
            )
            .await
            .expect("an attachment read in flight finishes")
            .expect("the attachment channel is open");
            self.app.apply_attachment_read(read);
        }
        self.capture();
        self
    }

    /// Whether an attachment read is in flight (it has not been applied).
    pub fn attachment_reads_in_flight(&self) -> bool {
        self.app.attachments.reads_in_flight()
    }

    /// The pending attachments' chips, as the pending list words them.
    pub fn attachment_chips(&self) -> Vec<String> {
        self.app.attachments.pending.chips()
    }

    /// Resolve workspace-relative `/image` paths against `root`.
    pub fn set_workspace_root(&mut self, root: std::path::PathBuf) -> &mut Self {
        self.app.workspace.root = Some(root);
        self
    }
}
