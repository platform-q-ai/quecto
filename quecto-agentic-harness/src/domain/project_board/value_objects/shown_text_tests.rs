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
