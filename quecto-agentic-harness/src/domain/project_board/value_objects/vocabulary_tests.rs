use super::*;

#[test]
fn every_word_reads_back_and_nothing_else_parses() {
    for status in TaskStatus::ALL {
        assert_eq!(TaskStatus::parse(status.as_str()), Some(*status));
    }
    for kind in TaskKind::ALL {
        assert_eq!(TaskKind::parse(kind.as_str()), Some(*kind));
    }
    let spelt: Vec<_> = TaskStatus::ALL
        .iter()
        .map(|status| status.as_str())
        .collect();
    assert_eq!(
        spelt.join(" "),
        "draft ready claimed in_progress review blocked done archived"
    );
    for bad in ["", "Ready", "in-progress", "wip", "story", " task"] {
        assert_eq!(
            (TaskStatus::parse(bad), TaskKind::parse(bad)),
            (None, None),
            "{bad:?}"
        );
    }
}

#[test]
fn the_parts_vocabularies_are_spelt_exactly() {
    let spelt = |words: Vec<&str>| words.join(" ");
    assert_eq!(
        spelt(ItemState::ALL.iter().map(|w| w.as_str()).collect()),
        "todo doing review done blocked"
    );
    assert_eq!(
        spelt(Effort::ALL.iter().map(|w| w.as_str()).collect()),
        "low medium high max"
    );
    assert_eq!(
        spelt(RunOutcome::ALL.iter().map(|w| w.as_str()).collect()),
        "succeeded failed stopped lost"
    );
    assert_eq!(
        spelt(Severity::ALL.iter().map(|w| w.as_str()).collect()),
        "low medium high critical"
    );
    assert_eq!(
        spelt(Resolution::ALL.iter().map(|w| w.as_str()).collect()),
        "completed not_planned duplicate superseded"
    );
    assert_eq!(
        spelt(DiscussionState::ALL.iter().map(|w| w.as_str()).collect()),
        "open closed"
    );
    assert_eq!(
        spelt(DiscussionLimit::ALL.iter().map(|w| w.as_str()).collect()),
        "messages lines text"
    );
    assert_eq!(
        (
            Effort::parse("xhigh"),
            Effort::parse("none"),
            Severity::parse("blocker")
        ),
        (None, None, None)
    );
}

/// A dependency done or archived (finished or cancelled) releases its
/// dependents; in any other status it holds them back.
#[test]
fn only_a_done_or_archived_dependency_releases_its_dependents() {
    let releasing: Vec<_> = TaskStatus::ALL
        .iter()
        .copied()
        .filter(|status| status.releases_dependents())
        .collect();
    assert_eq!(releasing, [TaskStatus::Done, TaskStatus::Archived]);
}
