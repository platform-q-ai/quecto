use super::{EXTERNAL_AGENT_STDERR_TAIL_BYTES, ExternalAgentExit, ExternalAgentInputError};

#[test]
fn the_stderr_tail_keeps_64_kib() {
    assert_eq!(EXTERNAL_AGENT_STDERR_TAIL_BYTES, 65_536);
}

#[test]
fn only_a_zero_exit_status_is_clean() {
    assert!(ExternalAgentExit::Code(0).is_clean());
    for unclean in [
        ExternalAgentExit::Code(1),
        ExternalAgentExit::Code(-1),
        ExternalAgentExit::Signal(0),
        ExternalAgentExit::Signal(9),
        ExternalAgentExit::Unobservable("gone".into()),
    ] {
        assert!(!unclean.is_clean(), "{unclean:?}");
    }
}

#[test]
fn an_input_error_says_what_went_wrong() {
    assert_eq!(
        ExternalAgentInputError::Closed.to_string(),
        "the external agent's input is closed"
    );
    assert_eq!(
        ExternalAgentInputError::Write("broken pipe".into()).to_string(),
        "writing to the external agent failed: broken pipe"
    );
}
