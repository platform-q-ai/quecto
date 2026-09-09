use super::*;

impl AgentSession {
    pub(crate) fn record_control(
        &mut self,
        id: Option<&str>,
        command: &str,
        status: crate::interface::cli::protocol::ControlStatus,
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
                .push(crate::interface::cli::protocol::ControlReceipt {
                    id: id.into(),
                    command: command.into(),
                    status,
                });
        }
        self.bump_visible_generation();
    }

    pub(crate) fn enqueue_control(
        &mut self,
        id: Option<&str>,
        command: &str,
        content: String,
        steer: bool,
    ) -> bool {
        if self.pending.len() < Self::MAX_PENDING {
            let message = match id {
                Some(id) => PendingMessage::Control {
                    id: id.into(),
                    command: command.into(),
                    content,
                },
                None => PendingMessage::User(content),
            };
            if steer {
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
