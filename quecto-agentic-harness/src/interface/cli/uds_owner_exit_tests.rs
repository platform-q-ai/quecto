use super::*;
use crate::infrastructure::tools::owner_exit::OwnerExitFlag;

#[test]
fn only_the_exit_persist_announces_and_it_is_still_forwarded() {
    for (line, expected) in [
        (
            r#"{"type":"persist_session","id":"x","restoreReason":"ordinary_tui_exit_stopped"}"#,
            true,
        ),
        (r#"{"type":"persist_session","id":"x"}"#, false),
        (
            r#"{"type":"persist_session","restoreReason":"something_else"}"#,
            false,
        ),
        (
            r#"{"type":"prompt","restoreReason":"ordinary_tui_exit_stopped"}"#,
            false,
        ),
        ("not json", false),
    ] {
        let flag = OwnerExitFlag::new();
        assert_eq!(
            announce_if_exit_persist(&*flag, 4, line),
            expected,
            "{line}"
        );
        assert_eq!(flag.announced(), expected, "{line}");
    }
}
