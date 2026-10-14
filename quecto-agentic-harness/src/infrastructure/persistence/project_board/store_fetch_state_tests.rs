//! The fetch state's file is read only when it is exactly its shape, of
//! this quecto's filter limit, and within its bounds.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::FILTER_BYTES;

fn header() -> String {
    format!("quecto-board-fetch 1 {FILTER_BYTES}\n")
}

#[test]
fn only_a_state_of_exactly_its_shape_is_read() {
    let (commit, blob) = ("c".repeat(40), "b".repeat(40));
    let state = FetchState {
        read: Some(commit.clone()),
        omitted: BTreeSet::from([blob.clone()]),
    };
    assert!(state.render().starts_with(&header()));
    assert_eq!(FetchState::parse(&state.render()), Some(state));
    assert_eq!(FetchState::parse(&header()), Some(FetchState::default()));
    for text in [
        String::new(),
        "quecto-board-fetch 1\n".to_string(),
        format!("quecto-board-fetch 1 {}\n", FILTER_BYTES + 1),
        format!("quecto-board-fetch 2 {FILTER_BYTES}\nread {commit}\n"),
        format!("{}read {commit}\nread {commit}\n", header()),
        format!("{}omitted {blob}\nread {commit}\n", header()),
        format!("{}read HEAD\n", header()),
        format!("{}kept {blob}\n", header()),
    ] {
        assert_eq!(FetchState::parse(&text), None, "{text:?}");
    }
}

/// The most a state holds: a commit and a blob for each entry a read
/// takes in `tasks/` and in all the folders; its bytes at most that many
/// lines of the longest ids.
#[test]
fn a_state_is_read_within_its_bounds() {
    let id = |n: usize, width: usize| format!("{n:0width$x}");
    let omitted = |count: usize, width: usize| -> String {
        (0..count)
            .map(|n| format!("omitted {}\n", id(n, width)))
            .collect()
    };
    let longest = format!(
        "{}read {}\n{}",
        header(),
        id(0, 64),
        omitted(MAX_STATE_ENTRIES, 64)
    );
    assert_eq!(longest.len(), MAX_STATE_BYTES);
    assert!(read_bounded(longest.as_bytes()).is_some());
    let one_more = format!("{}{}", header(), omitted(MAX_STATE_ENTRIES + 1, 40));
    assert!(one_more.len() < MAX_STATE_BYTES);
    assert_eq!(read_bounded(one_more.as_bytes()), None);
    let past = format!("{longest} ");
    assert_eq!(read_bounded(past.as_bytes()), None);
}

/// Each draft a process makes has a name of its own, whatever store or
/// thread makes it.
#[test]
fn every_draft_name_is_its_own() {
    let names: BTreeSet<String> = (0..64)
        .map(|n| match n % 2 {
            0 => draft_name("origin"),
            _ => std::thread::spawn(|| draft_name("origin")).join().unwrap(),
        })
        .collect();
    assert_eq!(names.len(), 64);
    let pid = std::process::id().to_string();
    for name in &names {
        let fields: Vec<&str> = name.split('.').collect();
        assert!(
            matches!(fields.as_slice(), ["fetch-origin", p, n, "draft"] if *p == pid && n.bytes().all(|b| b.is_ascii_digit())),
            "{name}"
        );
    }
}
