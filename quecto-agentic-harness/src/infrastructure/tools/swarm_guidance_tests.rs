use super::*;

#[test]
fn a_run_refused_during_setup_names_op_create_and_what_is_allowed() {
    let message = run_refused("setup");
    assert!(message.contains(r#""op":"create""#), "{message}");
    assert!(message.contains("Allowed now: create"), "{message}");
    assert!(
        !message.contains("\"setup\""),
        "the status is not quoted: {message}"
    );
}

#[test]
fn a_run_refused_when_paused_or_terminal_says_what_is_allowed() {
    assert!(run_refused("paused").contains("supervisor"));
    let ended = run_refused("succeeded");
    assert!(
        ended.contains("succeeded") && ended.contains("Allowed: summary"),
        "{ended}"
    );
}

#[test]
fn an_unknown_op_lists_the_valid_ones_and_how_to_end_a_run() {
    let message = unknown_op("stop");
    for op in VALID_OPS {
        assert!(message.contains(op), "{op}: {message}");
    }
    assert!(message.contains("board.stop"), "{message}");
}

#[test]
fn the_tool_description_shows_a_create_example() {
    let description = include_str!("swarm_helpers/tool_description.txt");
    assert!(
        description.contains(r#"{"op":"create""#),
        "no op=create example"
    );
}
