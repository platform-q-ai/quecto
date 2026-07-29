//! Command classification and harness support for the UDS client.
//!
//! Split from `client.rs` (750-line baseline): the writer-queue admission
//! classes (#1238 reserve, feed-liveness bypass) and the test-harness queue
//! constructor. A submodule of `client` so it can touch private fields.

use super::Command;
#[cfg(feature = "test-harness")]
use super::{COMMAND_WRITER_QUEUE_CAPACITY, CommandSender, mpsc};

/// Slots reserved so interactive user commands can still enqueue when
/// background fan-in has filled most of the ordered writer FIFO (#1238).
///
/// Background / housekeeping commands refuse to consume these last permits;
/// [`Command::is_interactive_user`] commands may use them. This does not
/// await capacity or reorder the FIFO — it only stops background traffic
/// from monopolizing the bound under sustained load.
pub const COMMAND_WRITER_USER_RESERVED: usize = 64;

/// The inner core of the user reserve that ONLY interactive commands may use.
///
/// Feed-liveness traffic ([`Command::is_feed_liveness`], i.e. `Sync`) is
/// admitted into the outer half of the reserve — refusing it under pressure
/// froze child feeds exactly while the parent was busy — but never past this
/// floor, so an unthrottled sync burst can not consume the slots protecting
/// prompt/steer/follow_up/abort (#1238, PR #1307 review).
pub const COMMAND_WRITER_INTERACTIVE_FLOOR: usize = 32;

impl Command {
    /// Interactive user actions that must not lose to background fan-in on
    /// the shared ordered writer queue (#1238).
    pub fn is_interactive_user(&self) -> bool {
        matches!(
            self,
            Self::Prompt { .. } | Self::Steer { .. } | Self::FollowUp { .. } | Self::Abort { .. }
        )
    }

    /// Commands that keep a feed live and must not be refused by the
    /// background reserve: a dropped `Sync` freezes the child feed until the
    /// next `ledger_advanced` (which may not come until the parent goes idle).
    pub fn is_feed_liveness(&self) -> bool {
        matches!(self, Self::Sync { .. })
    }
}

#[cfg(feature = "test-harness")]
impl CommandSender {
    /// A sender over a production-sized writer queue, plus its receiver, for
    /// BDD/harness tests that pin the backpressure-reserve semantics.
    pub fn production_queue_for_tests() -> (Self, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel::<String>(COMMAND_WRITER_QUEUE_CAPACITY);
        (Self { tx }, rx)
    }
}
