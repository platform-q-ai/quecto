//! Test-only `tracing` warn-capture apparatus, shared between the client
//! defence unit tests and the workspace `bdd` target (#1112 review).
//!
//! Both targets assert the same warn contract (an oversized-frame drop emits
//! exactly one `tracing::warn!` with a pinned message), so the capture
//! subscriber and the message constants live here once: a change to the warn
//! contract updates every consumer or compiles neither, instead of leaving a
//! stale copy green-lighting a regression.
//!
//! Compiled only for `cfg(test)` (lib unit tests) or the `test-harness`
//! feature (the `bdd` integration target, which cannot see `cfg(test)` items).

/// The exact message the client must warn with when dropping an oversized
/// inbound frame (mirrors quecto-api's UDS client, #1112).
pub const OVERSIZED_WARN_MSG: &str = "dropping oversized message from agent";

/// Captured `(level, message)` tracing events, shared with a [`WarnCapture`].
pub type CapturedEvents = std::sync::Arc<std::sync::Mutex<Vec<(tracing::Level, String)>>>;

/// How many captured events are exactly-WARN events whose message contains
/// `needle`. The exact-level match rules out a `warn!` → `error!` regression.
pub fn warn_count_containing(captured: &[(tracing::Level, String)], needle: &str) -> usize {
    captured
        .iter()
        .filter(|(level, msg)| *level == tracing::Level::WARN && msg.contains(needle))
        .count()
}

/// The exact message the client must warn with when dropping an oversized
/// outbound command (#1125).
pub const OVERSIZED_OUTBOUND_WARN_MSG: &str = "dropping oversized outbound command";

/// How many captured events are exactly-WARN inbound oversized-drop warnings.
pub fn oversized_warn_count(captured: &[(tracing::Level, String)]) -> usize {
    warn_count_containing(captured, OVERSIZED_WARN_MSG)
}

/// How many captured events are exactly-WARN outbound oversized-drop warnings.
pub fn oversized_outbound_warn_count(captured: &[(tracing::Level, String)]) -> usize {
    warn_count_containing(captured, OVERSIZED_OUTBOUND_WARN_MSG)
}

/// Install a fresh [`WarnCapture`] as the *thread-scoped* tracing default.
/// Returns the shared capture buffer and the guard keeping it installed.
pub fn install_warn_capture() -> (CapturedEvents, tracing::dispatcher::DefaultGuard) {
    let captured: CapturedEvents = std::sync::Arc::default();
    let dispatch = tracing::Dispatch::new(WarnCapture(std::sync::Arc::clone(&captured)));
    let guard = tracing::dispatcher::set_default(&dispatch);
    (captured, guard)
}

/// Minimal `tracing` subscriber that records the level and `message` field of
/// every WARN-or-more-severe event. Hand-rolled on the `tracing` core API so
/// the tests need no extra capture crate. ERROR events are captured too so a
/// `warn!` → `error!` regression shows up in assertion failure output.
pub struct WarnCapture(pub CapturedEvents);

struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl tracing::Subscriber for WarnCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::WARN
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap()
            .push((*event.metadata().level(), visitor.0));
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}
