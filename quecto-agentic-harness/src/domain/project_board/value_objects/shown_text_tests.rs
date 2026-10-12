use super::*;

#[test]
fn a_path_shown_holds_no_control_or_format_character() {
    assert!(shown_path("tasks/token=x.json"));
    assert!(shown_path("tasks/é ✓ ….json"));
    for hostile in [
        "README.md",
        "tasks/a\u{0}.json",
        "tasks/a\u{1b}[31m.json",
        "tasks/a\u{a0}.json",
        "tasks/a\u{202e}nosj.exe",
        "tasks/a\u{200b}.json",
        "tasks/a\u{e0041}.json",
        "tasks/a\u{feff}.json",
    ] {
        assert!(!shown_path(hostile), "{hostile:?}");
    }
}

#[test]
fn an_artifact_path_shown_is_under_its_kinds_folder() {
    for shown in [
        "plans/a.1.md",
        "runs/a.1.md",
        "changes/a.1.md",
        "reviews/a.1.md",
        "discussions/d.md",
    ] {
        assert!(shown_artifact_path(shown), "{shown:?}");
    }
    for hostile in [
        "tasks/a.md",
        "archive/a.md",
        "runs",
        "runs/a\u{202e}.md",
        "runs/a\u{0}.md",
    ] {
        assert!(!shown_artifact_path(hostile), "{hostile:?}");
    }
}
