use super::*;

#[test]
fn a_path_shown_holds_no_control_or_format_character() {
    assert!(shown_path("tasks/token=x.md"));
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
fn an_artifact_path_shown_is_one_name_in_its_kinds_folder() {
    for (folder, shown) in [
        ("plans", "plans/a.1.md"),
        ("runs", "runs/a.1.md"),
        ("changes", "changes/a.1.md"),
        ("reviews", "reviews/a.1.md"),
        ("discussions", "discussions/d.md"),
    ] {
        assert!(shown_artifact_path(folder, shown), "{shown:?}");
    }
    for (folder, hostile) in [
        ("runs", "plans/a.1.md"),
        ("tasks", "tasks/a.md"),
        ("archive", "archive/a.md"),
        ("runs", "runs"),
        ("runs", "runs/"),
        ("runs", "runsx/a.1.md"),
        ("runs", "runs/a/1.md"),
        ("runs", "runs/../tasks/a.md"),
        ("runs", "runs/a..1.md"),
        ("runs", "runs/a\u{202e}.md"),
        ("runs", "runs/a\u{0}.md"),
    ] {
        assert!(!shown_artifact_path(folder, hostile), "{hostile:?}");
    }
}

#[test]
fn a_task_path_shown_is_one_name_under_tasks() {
    for hostile in ["tasks/", "tasks/a/b.json", "tasks/..", "tasks/."] {
        assert!(!shown_path(hostile), "{hostile:?}");
    }
    assert!(shown_path("tasks/a..b.json"), "a name may hold two dots");
}

/// A path a store sets aside with no key (a stray) is one name directly
/// under any folder it reads: `tasks/` or another kind's folder.
#[test]
fn a_board_path_shown_is_one_name_in_any_folder_read() {
    for shown in [
        "tasks/a.md",
        "plans/notes.txt",
        "runs/a..b.txt",
        "changes/x",
        "reviews/a.1.md",
        "discussions/d.1.md",
    ] {
        assert!(shown_board_path(shown), "{shown:?}");
    }
    for hostile in [
        "archive/a.md",
        "format/board",
        "src/a.md",
        "README.md",
        "plans/",
        "plans/..",
        "plans/a/b.md",
        "plansx/a.md",
        "plans/a\u{202e}.md",
    ] {
        assert!(!shown_board_path(hostile), "{hostile:?}");
    }
}
