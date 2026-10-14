//! The fetch state's file is read only when it is exactly its shape.
use super::*;

#[test]
fn only_a_state_of_exactly_its_shape_is_read() {
    let (commit, blob) = ("c".repeat(40), "b".repeat(40));
    let state = FetchState {
        read: Some(commit.clone()),
        omitted: BTreeSet::from([blob.clone()]),
    };
    assert_eq!(FetchState::parse(&state.render()), Some(state));
    assert_eq!(
        FetchState::parse(&format!("{HEADER}\n")),
        Some(FetchState::default())
    );
    for text in [
        String::new(),
        format!("quecto-board-fetch 2\nread {commit}\n"),
        format!("{HEADER}\nread {commit}\nread {commit}\n"),
        format!("{HEADER}\nomitted {blob}\nread {commit}\n"),
        format!("{HEADER}\nread HEAD\n"),
        format!("{HEADER}\nkept {blob}\n"),
    ] {
        assert_eq!(FetchState::parse(&text), None, "{text:?}");
    }
}
