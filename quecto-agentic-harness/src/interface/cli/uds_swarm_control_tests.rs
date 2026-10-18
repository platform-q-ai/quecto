//! #2478 slice B: the wake prompts members read name task items, as the
//! owner approved them (2026-10-12).
use super::{CHANGED_WAKE, RESUMED_WAKE};

#[test]
fn the_wake_prompts_read_task_items() {
    assert_eq!(
        RESUMED_WAKE,
        "The swarm was resumed after a provider failure interrupted your turn. Continue your \
         interrupted work: inspect summary using your last event_cursor, read and acknowledge \
         relevant durable inbox messages, work only on actionable task items and yield when none \
         remain."
    );
    assert_eq!(
        CHANGED_WAKE,
        "Swarm work changed. Inspect summary using your last event_cursor, then read and \
         acknowledge relevant durable inbox messages. Work only on actionable task items; yield \
         when none remain."
    );
}
