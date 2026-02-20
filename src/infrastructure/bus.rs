// Async message bus: tokio channels for inbound/outbound message routing.

use tokio::sync::mpsc;

/// An outbound message to be sent to a channel.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    /// Target in the form "channel:chat_id" (e.g. "telegram:12345").
    pub target: String,
    /// The text content to send.
    pub text: String,
}

/// An inbound message received from a channel.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// Source in the form "channel:chat_id" (e.g. "telegram:12345").
    pub source: String,
    /// Sender identifier.
    pub sender_id: String,
    /// The text content received.
    pub text: String,
}

/// The message bus provides channels for routing messages between
/// the agent/gateway and channel adapters (e.g. Telegram).
#[derive(Debug)]
pub struct MessageBus {
    outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: Option<mpsc::Receiver<OutboundMessage>>,
    inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: Option<mpsc::Receiver<InboundMessage>>,
}

impl MessageBus {
    /// Create a new message bus with the given channel buffer size.
    pub fn new(buffer: usize) -> Self {
        let (outbound_tx, outbound_rx) = mpsc::channel(buffer);
        let (inbound_tx, inbound_rx) = mpsc::channel(buffer);
        Self {
            outbound_tx,
            outbound_rx: Some(outbound_rx),
            inbound_tx,
            inbound_rx: Some(inbound_rx),
        }
    }

    /// Get a sender for outbound messages (used by tools like `message`).
    pub fn outbound_sender(&self) -> mpsc::Sender<OutboundMessage> {
        self.outbound_tx.clone()
    }

    /// Take the outbound receiver (used by the gateway to dispatch to channels).
    /// Can only be called once.
    pub fn take_outbound_receiver(&mut self) -> Option<mpsc::Receiver<OutboundMessage>> {
        self.outbound_rx.take()
    }

    /// Get a sender for inbound messages (used by channel adapters like Telegram).
    pub fn inbound_sender(&self) -> mpsc::Sender<InboundMessage> {
        self.inbound_tx.clone()
    }

    /// Take the inbound receiver (used by the gateway/agent loop to process messages).
    /// Can only be called once.
    pub fn take_inbound_receiver(&mut self) -> Option<mpsc::Receiver<InboundMessage>> {
        self.inbound_rx.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_outbound_message_routing() {
        let mut bus = MessageBus::new(16);
        let sender = bus.outbound_sender();
        let mut receiver = bus.take_outbound_receiver().unwrap();

        sender
            .send(OutboundMessage {
                target: "telegram:12345".to_string(),
                text: "Hello!".to_string(),
            })
            .await
            .unwrap();

        let msg = receiver.recv().await.unwrap();
        assert_eq!(msg.target, "telegram:12345");
        assert_eq!(msg.text, "Hello!");
    }

    #[tokio::test]
    async fn test_inbound_message_routing() {
        let mut bus = MessageBus::new(16);
        let sender = bus.inbound_sender();
        let mut receiver = bus.take_inbound_receiver().unwrap();

        sender
            .send(InboundMessage {
                source: "telegram:12345".to_string(),
                sender_id: "user42".to_string(),
                text: "Hi bot".to_string(),
            })
            .await
            .unwrap();

        let msg = receiver.recv().await.unwrap();
        assert_eq!(msg.source, "telegram:12345");
        assert_eq!(msg.sender_id, "user42");
        assert_eq!(msg.text, "Hi bot");
    }

    #[test]
    fn test_take_receiver_only_once() {
        let mut bus = MessageBus::new(16);
        assert!(bus.take_outbound_receiver().is_some());
        assert!(bus.take_outbound_receiver().is_none());
    }

    #[tokio::test]
    async fn test_multiple_outbound_messages() {
        let mut bus = MessageBus::new(16);
        let sender = bus.outbound_sender();
        let mut receiver = bus.take_outbound_receiver().unwrap();

        for i in 0..3 {
            sender
                .send(OutboundMessage {
                    target: format!("telegram:{}", i),
                    text: format!("msg {}", i),
                })
                .await
                .unwrap();
        }

        for i in 0..3 {
            let msg = receiver.recv().await.unwrap();
            assert_eq!(msg.target, format!("telegram:{}", i));
        }
    }
}
