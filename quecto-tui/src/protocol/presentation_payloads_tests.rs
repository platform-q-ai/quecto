//! Unit tests for `presentation_payloads` mappings.

use super::is_subagent_note;

#[test]
fn detects_notes_verbatim_and_collapsed() {
    assert!(is_subagent_note(
        "<subagent_notification source=\"spawn_tool\" agent_id=\"poet\" sequence=\"1\">\nidle\n</subagent_notification>"
    ));
    // Ladder-collapsed form (context_pruning::message_collapse_stub).
    assert!(is_subagent_note(
        "[user: \"<subagent_notification source=\"spawn_tool\" agent_id=\"po\" (31 tokens) — recall(\"turn3:msg:user\")]"
    ));
    assert!(!is_subagent_note("write me a poem"));
    assert!(!is_subagent_note(
        "[user: \"write me a poem\" (4 tokens) — recall(\"turn1:msg:user\")]"
    ));
}
