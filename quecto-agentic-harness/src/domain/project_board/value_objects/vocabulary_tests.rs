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
