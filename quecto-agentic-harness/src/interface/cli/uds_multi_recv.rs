//! Next-message selection for the multi-client dispatch loop: shutdown first,
//! then client commands, then disconnect sentinels (#1720), then sub-agent
//! notifications. Child module of `uds_multi`.
use super::{ClientDisconnected, ClientMessage};

pub(super) enum DispatchMsg {
    Client(ClientMessage),
    Notification(crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification),
    /// A termination signal was handled and its teardown has completed.
    Shutdown,
}

pub(super) async fn recv_next_message(
    cmd_rx: &mut tokio::sync::mpsc::Receiver<ClientMessage>,
    disconnect_rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientDisconnected>,
    notification_rx: &mut Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    shutdown: &super::super::uds_shutdown::ShutdownRequest,
) -> Option<DispatchMsg> {
    // Commands outrank disconnect sentinels: a client's queued work (an
    // acknowledged follow-up, a tool registration) is handled before its
    // own disconnect, as it was when both shared one channel, so the
    // last-client exit and tool bookkeeping never overtake admitted
    // commands (#1720). Sentinels drain whenever the command channel is idle.
    if let Some(rx) = notification_rx {
        tokio::select! {
            biased;
            () = shutdown.requested() => Some(DispatchMsg::Shutdown),
            client_msg = cmd_rx.recv() => client_msg.map(DispatchMsg::Client),
            Some(disc) = disconnect_rx.recv() => Some(DispatchMsg::Client(ClientMessage::Disconnected(disc))),
            Some(notif) = rx.recv() => Some(DispatchMsg::Notification(notif)),
        }
    } else {
        tokio::select! {
            biased;
            () = shutdown.requested() => Some(DispatchMsg::Shutdown),
            client_msg = cmd_rx.recv() => client_msg.map(DispatchMsg::Client),
            Some(disc) = disconnect_rx.recv() => Some(DispatchMsg::Client(ClientMessage::Disconnected(disc))),
        }
    }
}
