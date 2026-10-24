//! Control receipts and pending-queue control of the [`AgentSession`]
//! tracker (steer, follow-up, abort bookkeeping). Owner role: control.
//! Child module of `uds_session`; holds no session persistence.
use super::*;

impl AgentSession {
    pub(crate) fn record_control(
        &mut self,
        id: Option<&str>,
        command: &str,
        status: crate::interface::uds::wire::protocol::ControlStatus,
    ) {
        let Some(id) = id else {
            return;
        };
        if let Some(receipt) = self
            .control_receipts
            .iter_mut()
            .find(|receipt| receipt.id == id && receipt.command == command)
        {
            receipt.status = status;
        } else {
            if self.control_receipts.len() == Self::MAX_PENDING {
                self.control_receipts.remove(0);
            }
            self.control_receipts
                .push(crate::interface::uds::wire::protocol::ControlReceipt {
                    id: id.into(),
                    command: command.into(),
                    status,
                });
        }
        self.bump_visible_generation();
    }

    /// The latest recorded status of a control receipt, by id.
    #[cfg(test)]
    pub(crate) fn control_receipt_status(
        &self,
        id: &str,
    ) -> Option<crate::interface::uds::wire::protocol::ControlStatus> {
        self.control_receipts
            .iter()
            .rev()
            .find(|receipt| receipt.id == id)
            .map(|receipt| receipt.status)
    }

    pub(crate) fn enqueue_control(
        &mut self,
        id: Option<&str>,
        command: &str,
        body: super::PromptBody,
        work: crate::domain::inference::services::turn_suspension::Work,
    ) -> bool {
        use crate::domain::inference::services::turn_suspension::Work;
        if self.pending.len() < Self::MAX_PENDING {
            let super::PromptBody {
                text: content,
                images,
            } = body;
            // The work the caller knows (#1909), never re-derived from the
            // command name.
            let message = match (work, id) {
                (Work::Instruction(_), Some(id)) => PendingMessage::Control {
                    id: id.into(),
                    command: command.into(),
                    content,
                    images,
                },
                (Work::Instruction(_), None) => PendingMessage::User { content, images },
                // A harness nudge carries no id, no human intent, no images.
                (Work::Automatic, None) => {
                    assert!(images.is_empty(), "automatic work carries no images");
                    PendingMessage::Automatic(content)
                }
                (Work::Automatic, Some(id)) => {
                    panic!("automatic work carries no correlation id ({id})")
                }
            };
            if work.is_steer() {
                self.pending.push_front(message);
            } else {
                self.pending.push_back(message);
            }
            true
        } else {
            false
        }
    }
}
