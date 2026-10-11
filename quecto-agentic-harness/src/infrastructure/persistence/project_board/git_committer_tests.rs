use super::{ConfigAnswer, config_answer};

#[test]
fn only_exit_one_with_no_message_is_unset() {
    for (code, stdout, stderr, answer) in [
        (Some(0), "Ada\n", "", ConfigAnswer::Set("Ada".into())),
        (
            Some(0),
            "Ada\n",
            "warning: x",
            ConfigAnswer::Set("Ada".into()),
        ),
        (Some(1), "", "", ConfigAnswer::Unset),
        (
            Some(1),
            "",
            "error: bad config line 3",
            ConfigAnswer::Failed,
        ),
        (
            Some(128),
            "",
            "fatal: not a git repository",
            ConfigAnswer::Failed,
        ),
        (Some(2), "", "", ConfigAnswer::Failed),
        (None, "", "", ConfigAnswer::Failed),
    ] {
        assert_eq!(
            config_answer(code, stdout.as_bytes(), stderr.as_bytes()),
            answer,
            "{code:?} {stderr:?}"
        );
    }
}
