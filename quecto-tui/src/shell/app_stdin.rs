use super::*;

impl App {
    pub(super) async fn process_stdin_bytes(
        &mut self,
        bytes: Vec<u8>,
        stdin_rx: &mut mpsc::Receiver<Vec<u8>>,
        escape_timeout: Duration,
        kitty_fallback_done: &mut bool,
    ) -> bool {
        // Check for Kitty protocol response before buffering.
        if !self.kitty.active && !*kitty_fallback_done {
            if let Some(_flags) = KittyProtocol::parse_response(&bytes) {
                self.kitty.enable();
                *kitty_fallback_done = true;
                return false;
            }
        }

        // Feed bytes into the reason-aware decoder. A rejected feed discards
        // the whole staged event; it is never safe to submit a truncated paste.
        // This entry point represents a fresh user-visible input action; reads
        // coalesced below remain part of the active burst.
        self.stdin_buffer.begin_input_action(&bytes);
        self.stdin_buffer.feed(&bytes);
        self.drain_decoded_stdin_events();
        self.drain_pending_stdin_bytes(stdin_rx, escape_timeout)
            .await;
        true
    }

    async fn drain_pending_stdin_bytes(
        &mut self,
        stdin_rx: &mut mpsc::Receiver<Vec<u8>>,
        escape_timeout: Duration,
    ) {
        use crate::shell::stdin_buffer::PendingReason;

        while !self.should_exit {
            let Some(reason) = self.stdin_buffer.pending_reason() else {
                return;
            };

            match reason {
                PendingReason::BracketedPaste => {
                    // A real bracketed paste is marker-delimited, not timed or
                    // read-count-delimited. Consume everything already queued,
                    // then return control to the outer select loop until the
                    // exact ESC[201~ marker arrives in a later stdin read.
                    match stdin_rx.try_recv() {
                        Ok(more) => {
                            self.stdin_buffer.feed(&more);
                            self.drain_decoded_stdin_events();
                        }
                        Err(mpsc::error::TryRecvError::Empty) => return,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            let events = self.stdin_buffer.finish_pending(true);
                            self.process_stdin_events(events);
                            return;
                        }
                    }
                }
                PendingReason::RawCandidate | PendingReason::RawPaste => {
                    // Every fragment extends the raw burst's quiet boundary.
                    // There is deliberately no escape retry/read-count cap.
                    match tokio::time::timeout(RAW_PASTE_QUIET_TIMEOUT, stdin_rx.recv()).await {
                        Ok(Some(more)) => {
                            self.stdin_buffer.feed(&more);
                            self.drain_decoded_stdin_events();
                        }
                        Ok(None) => {
                            let events = self.stdin_buffer.finish_pending(true);
                            self.process_stdin_events(events);
                            return;
                        }
                        Err(_) => {
                            let events = self.stdin_buffer.finish_pending(false);
                            self.process_stdin_events(events);
                            return;
                        }
                    }
                }
                PendingReason::Escape | PendingReason::Utf8 => {
                    // Only escape/UTF-8 disambiguation uses the short key
                    // timeout. Paste states can never enter this force path.
                    match tokio::time::timeout(escape_timeout, stdin_rx.recv()).await {
                        Ok(Some(more)) => {
                            self.stdin_buffer.feed(&more);
                            self.drain_decoded_stdin_events();
                        }
                        Ok(None) => {
                            let events = self.stdin_buffer.finish_pending(true);
                            self.process_stdin_events(events);
                            return;
                        }
                        Err(_) => {
                            let events = self.stdin_buffer.finish_pending(false);
                            self.process_stdin_events(events);
                            return;
                        }
                    }
                }
            }
        }
    }

    fn drain_decoded_stdin_events(&mut self) {
        let events = self.stdin_buffer.drain_events();
        self.process_stdin_events(events);
    }

    fn process_stdin_events(&mut self, events: Vec<crate::shell::stdin_buffer::InputEvent>) {
        use crate::shell::stdin_buffer::InputEvent;

        for event in events {
            match event {
                InputEvent::KeySequence(sequence) => self.process_key_sequence(&sequence),
                InputEvent::Paste(text) => self.handle_key(Key::Paste(text)),
                InputEvent::Overflow => self.notify(
                    "Input was too large; the entire staged paste was discarded",
                    NotifyLevel::Error,
                ),
                InputEvent::IncompleteBracketedPaste => self.notify(
                    "Incomplete bracketed paste was discarded at end of input",
                    NotifyLevel::Error,
                ),
            }
            if self.should_exit {
                break;
            }
        }
    }
}
