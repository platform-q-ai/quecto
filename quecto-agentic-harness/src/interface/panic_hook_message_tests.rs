//! Review round 4 (L5): a panic's message leaves the process through one
//! helper, whichever sink it reaches.
use super::*;

/// Review round 4 (L5): every sink shows a panic's message through the one
/// helper: secrets and URL userinfo redacted, at most 1 KiB. The stderr
/// line, a fatal report's struck panic and the crash record.
#[test]
fn every_sink_shows_a_panic_message_through_the_one_helper() {
    let leaky = PanicSite {
        message: format!(
            "token ghp_0123456789abcdef0123456789abcdef0123 at https://user:pw@host/x {}",
            "x".repeat(4096)
        ),
        location: None,
    };
    let shown = crate::application::agent_turn::agent_loop::tool_panic_scope::shown_panic_message(
        &leaky.message,
    );
    assert!(
        !shown.contains("ghp_0123456789") && !shown.contains("user:pw"),
        "{shown}"
    );
    assert!(shown.len() <= 1024 + '…'.len_utf8(), "{}", shown.len());
    let stderr = contained_report("edit", &leaky);
    let struck = fatal_context(&[], Some(&leaky)).unwrap();
    let record = crash(&leaky, None);
    for sink in [&stderr, &struck, &record.panic.message] {
        assert!(sink.contains(&shown), "{sink}");
        assert!(!sink.contains("user:pw") && !sink.contains("ghp_0123456789"));
    }
}
