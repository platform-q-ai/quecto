//! #2202: binary files a directory search skips are not skipped silently.
//! When the search finds nothing, a paths-only rg run names the binary
//! files holding a match (#2251 review); a search that found matches never
//! starts it.
use super::grep_binary::{Checked, listed_files, notice, stopped_files, stopped_notice};
use super::*;
use tempfile::TempDir;

/// A workspace holding `files` (made binary by a NUL byte where asked).
fn grep_in(files: &[(&str, &[u8])]) -> (GrepTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    for (name, content) in files {
        let path = tmp.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let ws = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    (GrepTool::new(ws, sandbox), tmp)
}

async fn search(tool: &GrepTool, args: serde_json::Value) -> String {
    let result = tool.execute(&args.to_string()).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    result.content
}

const BINARY: &[u8] = b"\x7fELF\x00\x01GLIBC_2.34\x00more\n";

/// The issue's case: a directory holding only a binary match.
#[tokio::test]
async fn no_matches_in_a_directory_says_a_binary_file_holds_one() {
    let (tool, _tmp) = grep_in(&[("bin/ls", BINARY)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "GLIBC", "path": "bin"}),
    )
    .await;
    assert!(out.starts_with("No matches found"), "{out}");
    assert!(
        out.contains(
            r#"1 binary file holds a match but was skipped: "bin/ls". Name it as path to see its matches"#
        ),
        "{out}"
    );
}

/// A search that found matches says nothing of binary files.
#[tokio::test]
async fn matches_found_say_nothing_of_binary_files() {
    let (tool, _tmp) = grep_in(&[
        ("a.bin", BINARY),
        ("notes.txt", b"GLIBC is the C library\n"),
    ]);
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(out.contains("notes.txt:1: GLIBC is the C library"), "{out}");
    assert!(!out.contains("binary"), "{out}");
}

/// Each output mode says what the skipped files are missing from.
#[tokio::test]
async fn listings_say_so_in_their_own_words() {
    let (tool, _tmp) = grep_in(&[("x.bin", BINARY), ("y.txt", b"other\n")]);
    let expected = [
        (
            "files",
            r#"1 binary file holds a match but was not listed: "x.bin"]"#,
        ),
        (
            "count",
            r#"1 binary file holds a match but was not counted: "x.bin". Name it as path to count its matches]"#,
        ),
    ];
    for (output, note) in expected {
        let out = search(
            &tool,
            serde_json::json!({"pattern": "GLIBC", "output": output}),
        )
        .await;
        assert!(out.starts_with("No matches found"), "{output}: {out}");
        assert!(out.contains(note), "{output}: {out}");
    }
}

/// A binary file named as the path is searched (rg's own rule), so there
/// is nothing to say.
#[tokio::test]
async fn a_binary_file_named_as_the_path_is_searched() {
    let (tool, _tmp) = grep_in(&[("x.bin", BINARY)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "GLIBC", "path": "x.bin", "output": "count"}),
    )
    .await;
    assert!(out.contains("x.bin: 1"), "{out}");
    assert!(!out.contains("binary"), "{out}");
}

/// Binary files without the pattern are not worth a word.
#[tokio::test]
async fn binary_files_without_a_match_are_not_mentioned() {
    let (tool, _tmp) = grep_in(&[("x.bin", BINARY), ("y.txt", b"hello\n")]);
    let out = search(&tool, serde_json::json!({"pattern": "absent"})).await;
    assert_eq!(out, "No matches found");
}

/// The check honours the search's own filters.
#[tokio::test]
async fn skipped_binary_files_are_counted_within_the_search_filters() {
    let (tool, _tmp) = grep_in(&[("x.bin", BINARY), ("x.dat", BINARY)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "GLIBC", "glob": "*.dat"}),
    )
    .await;
    assert!(out.contains(r#"skipped: "x.dat"."#), "{out}");
    assert!(!out.contains("x.bin"), "{out}");
}

/// The note stays bounded however many binary files match, and names the
/// first in path order.
#[tokio::test]
async fn many_skipped_binary_files_are_counted_and_a_few_named() {
    let names: Vec<String> = (0..12).map(|i| format!("f{i:02}.bin")).collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), BINARY)).collect();
    let (tool, _tmp) = grep_in(&files);
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(
        out.contains(r#"12 binary files hold matches but were skipped: "f00.bin", "f01.bin", "f02.bin" (and 9 more). Name one as path to see its matches"#),
        "{out}"
    );
}

/// rg's `--null` listing: complete records only, UTF-8 paths only.
#[test]
fn listed_files_are_read_from_complete_utf8_records() {
    let stdout = b"/ws/a.bin\0/ws/b\xff.bin\0\0/ws/c.bin\0/ws/cut";
    assert_eq!(listed_files(stdout), ["/ws/a.bin", "/ws/c.bin"]);
    assert!(listed_files(b"").is_empty());
}

fn found(n: usize, complete: bool) -> Checked {
    Checked::Found {
        files: (0..n).map(|i| format!("f{i}")).collect(),
        complete,
    }
}

const UNCHECKED: &str = "Binary files are skipped, and they could not all be checked for matches";

#[test]
fn the_notice_is_bounded_and_says_when_it_is_a_floor() {
    let content = OutputMode::Content;
    assert_eq!(notice(&found(0, true), content), None);
    assert_eq!(notice(&Checked::NotNeeded, content), None);
    assert_eq!(
        notice(&found(0, false), content).as_deref(),
        Some(UNCHECKED)
    );
    assert_eq!(
        notice(&found(1, true), content).as_deref(),
        Some(
            r#"1 binary file holds a match but was skipped: "f0". Name it as path to see its matches"#
        )
    );
    assert_eq!(
        notice(&found(4, true), content).as_deref(),
        Some(
            r#"4 binary files hold matches but were skipped: "f0", "f1", "f2" (and 1 more). Name one as path to see its matches"#
        )
    );
    // Cut short: a floor, and examples rather than "the first".
    assert_eq!(
        notice(&found(1, false), content).as_deref(),
        Some(
            r#"At least 1 binary file holds a match but was skipped, e.g. "f0". Name it as path to see its matches"#
        )
    );
    assert_eq!(
        notice(&found(5, false), content).as_deref(),
        Some(
            r#"At least 5 binary files hold matches but were skipped, e.g. "f0", "f1", "f2". Name one as path to see its matches"#
        )
    );
    assert_eq!(
        notice(&found(2, true), OutputMode::Files).as_deref(),
        Some(r#"2 binary files hold matches but were not listed: "f0", "f1""#)
    );
    assert_eq!(
        notice(&found(2, true), OutputMode::Count).as_deref(),
        Some(
            r#"2 binary files hold matches but were not counted: "f0", "f1". Name one as path to count its matches"#
        )
    );
}

/// #2251 review (L4): a name holding `]`, a quote or a newline is quoted
/// and escaped, so it cannot end the note or break its line.
#[test]
fn names_in_the_notice_are_quoted_and_escaped() {
    let checked = Checked::Found {
        files: vec!["we]ird\n\"name\".bin".to_string()],
        complete: true,
    };
    let note = notice(&checked, OutputMode::Content).unwrap();
    assert!(note.contains(r#""we]ird\n\"name\".bin""#), "{note}");
    assert!(!note.contains('\n'), "{note}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_name_with_a_newline_is_shown_on_one_line() {
    let (tool, _tmp) = grep_in(&[("odd\nname].bin", BINARY)]);
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(
        out.contains(r#"skipped: "odd\nname].bin". Name it"#),
        "{out}"
    );
    assert_eq!(out.lines().count(), 3, "{out}");
}

/// A tool whose rg runs `check` (shell) for the binary check and the real
/// rg for the search itself, in a workspace holding `files`. Each check
/// started leaves a mark in `checks.log`.
#[cfg(unix)]
fn with_check(files: &[(&str, &[u8])], check: &str) -> (GrepTool, TempDir) {
    let (_, tmp) = grep_in(files);
    let script = tmp.path().join("check-rg");
    crate::infrastructure::test_support::executable::write_executable(
        &script,
        format!(
            "#!/bin/sh\ncase \" $* \" in *' --binary '*) echo check >> \"$(dirname \"$0\")/checks.log\"; {check};; esac\nexec rg \"$@\"\n"
        ),
    );
    let tool = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()))),
        script.to_string_lossy().into_owned(),
    );
    (tool, tmp)
}

/// How many binary checks the tool started.
fn checks_started(tmp: &TempDir) -> usize {
    std::fs::read_to_string(tmp.path().join("checks.log"))
        .map(|log| log.lines().count())
        .unwrap_or(0)
}

/// #2251 review (M1): the check reruns the search, so it runs only when the
/// search found nothing: never after matches, in any output mode.
#[cfg(unix)]
#[tokio::test]
async fn a_search_that_found_matches_never_starts_the_check() {
    let (tool, tmp) = with_check(
        &[("x.bin", BINARY), ("t.txt", b"GLIBC\n")],
        "exec rg \"$@\"",
    );
    for output in ["content", "files", "count"] {
        let out = search(
            &tool,
            serde_json::json!({"pattern": "GLIBC", "output": output}),
        )
        .await;
        assert!(out.contains("t.txt"), "{output}: {out}");
    }
    assert_eq!(checks_started(&tmp), 0);
    let out = search(&tool, serde_json::json!({"pattern": "ELF"})).await;
    assert!(out.contains(r#"skipped: "x.bin""#), "{out}");
    assert_eq!(checks_started(&tmp), 1);
}

/// Nor for a file named as the path, which rg searches whole.
#[cfg(unix)]
#[tokio::test]
async fn a_file_named_as_the_path_never_starts_the_check() {
    let (tool, tmp) = with_check(&[("t.txt", b"text\n")], "exec rg \"$@\"");
    let out = search(
        &tool,
        serde_json::json!({"pattern": "absent", "path": "t.txt"}),
    )
    .await;
    assert_eq!(out, "No matches found");
    assert_eq!(checks_started(&tmp), 0);
}

#[cfg(unix)]
/// Prints rg's `--null` listing of `names` in the workspace (where the
/// stand-in rg is), in the order given.
fn print_listed(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!(r#"printf '%s/{name}\0' "$(dirname "$0")""#))
        .collect::<Vec<_>>()
        .join("; ")
}

/// A check that outlasts its time is stopped, and the search is answered
/// all the same.
#[cfg(unix)]
#[tokio::test]
async fn a_slow_binary_check_does_not_hold_the_search() {
    let (tool, _tmp) = with_check(&[("t.txt", b"text\n")], "exec sleep 30");
    let started = std::time::Instant::now();
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert!(out.starts_with("No matches found"), "{out}");
    assert!(out.contains(UNCHECKED), "{out}");
}

/// A check that fails (here: stopped at the rg timeout having printed
/// nothing) found nothing, which is not "no binary file holds a match".
#[cfg(unix)]
#[tokio::test]
async fn a_failed_binary_check_does_not_claim_there_is_nothing_to_say() {
    let (tool, _tmp) = with_check(&[("t.txt", b"text\n")], "exec sleep 30");
    let tool = tool.with_rg_timeout(std::time::Duration::from_millis(500));
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(out.contains(UNCHECKED), "{out}");
}

/// #2251 review (L2): rg exiting 2 reported an error (an unreadable
/// file): the files it listed are a floor, as for any unfinished check.
#[cfg(unix)]
#[tokio::test]
async fn a_check_that_reported_errors_counts_at_least_what_it_found() {
    for code in [2, 3] {
        let (tool, _tmp) = with_check(
            &[("t.txt", b"text\n")],
            &format!("{}; exit {code}", print_listed(&["a.bin"])),
        );
        let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
        assert!(
            out.contains(r#"At least 1 binary file holds a match but was skipped, e.g. "a.bin"."#),
            "exit {code}: {out}"
        );
    }
}

/// A clean exit with everything read is complete.
#[cfg(unix)]
#[tokio::test]
async fn a_check_that_finished_counts_exactly() {
    let (tool, _tmp) = with_check(
        &[("t.txt", b"text\n")],
        &format!("{}; exit 0", print_listed(&["a.bin"])),
    );
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(
        out.contains(r#"1 binary file holds a match but was skipped: "a.bin"."#),
        "{out}"
    );
    assert!(!out.contains("At least"), "{out}");
}

/// #2251 review (L3): a check cut at its read cap is a floor, even when
/// rg itself exited cleanly (a descendant still writing), and the files it
/// did list are sorted: examples, not "the first".
#[cfg(unix)]
#[tokio::test]
async fn a_check_cut_short_counts_at_least_what_it_found_in_path_order() {
    let flood = "(head -c 6000000 /dev/zero | tr '\\0' x) & exit 0";
    let (tool, _tmp) = with_check(
        &[("t.txt", b"text\n")],
        &format!(
            "{}; {flood}",
            print_listed(&["d.bin", "b.bin", "a.bin", "c.bin"])
        ),
    );
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(
        out.contains(r#"At least 4 binary files hold matches but were skipped, e.g. "a.bin", "b.bin", "c.bin"."#),
        "{out}"
    );
}

/// The search log's directory is never searched, so its binary files are
/// never named either.
#[tokio::test]
async fn binary_files_in_an_excluded_directory_are_not_named() {
    let (tool, tmp) = grep_in(&[("log/x.bin", BINARY), ("y.bin", BINARY)]);
    let tool = tool.excluding(tmp.path().join("log"));
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(
        out.contains(r#"1 binary file holds a match but was skipped: "y.bin"."#),
        "{out}"
    );
    assert!(!out.contains("x.bin"), "{out}");
}

/// A search of "." names the files as the listing does, without `./`.
#[tokio::test]
async fn a_search_of_the_workspace_itself_names_files_relative_to_it() {
    let (tool, _tmp) = grep_in(&[("sub/x.bin", BINARY)]);
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC", "path": "."})).await;
    assert!(
        out.contains(r#"1 binary file holds a match but was skipped: "sub/x.bin"."#),
        "{out}"
    );
}

/// The check is the search with `--binary`, listing paths only (no line
/// content), whatever the output mode and per-file cap.
#[test]
fn the_check_lists_binary_files_by_path_only() {
    let request = super::grep_request::parse_request(&serde_json::json!({
        "pattern": "x", "output": "content", "maxPerFile": 7, "glob": "*.so"
    }))
    .unwrap();
    let args = |pass| {
        build_rg_command("rg", Path::new("/ws"), Path::new("/ws"), &request, pass)
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    let check = args(Pass::BinaryCheck);
    for flag in [
        "--binary",
        "--files-with-matches",
        "--null",
        "*.so",
        "--hidden",
    ] {
        assert!(check.contains(&flag.to_string()), "{flag}: {check:?}");
    }
    for flag in ["--json", "--max-count", "--count-matches"] {
        assert!(!check.contains(&flag.to_string()), "{flag}: {check:?}");
    }
    let search = args(Pass::Search);
    assert!(!search.contains(&"--binary".to_string()), "{search:?}");
    assert!(search.contains(&"--max-count".to_string()), "{search:?}");
}

/// A text file whose NUL byte comes after its first matches, past rg's
/// first read (so those matches were printed before rg saw the NUL).
fn partly_binary() -> Vec<u8> {
    let mut content = b"GLIBC one\n".to_vec();
    content.extend(std::iter::repeat_n(b'a', 200_000));
    content.extend_from_slice(b"\n\0GLIBC two\n");
    content
}

/// #2251 review (L1): a directory search stops reading a file at its NUL
/// byte, after the matches before it: the result says so.
#[tokio::test]
async fn a_search_stopped_at_a_nul_byte_after_matches_says_so() {
    let content = partly_binary();
    let (tool, _tmp) = grep_in(&[("partial.txt", &content), ("t.txt", b"GLIBC\n")]);
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(out.contains("partial.txt:1: GLIBC one"), "{out}");
    assert!(!out.contains("GLIBC two"), "{out}");
    assert!(
        out.contains(r#"rg stopped reading 1 file at a NUL byte, after its first matches: "partial.txt". Name it as path to search it whole"#),
        "{out}"
    );
    // Named as the path, it is searched whole, with nothing to say.
    let out = search(
        &tool,
        serde_json::json!({"pattern": "GLIBC", "path": "partial.txt"}),
    )
    .await;
    assert!(out.contains("GLIBC two"), "{out}");
    assert!(!out.contains("NUL"), "{out}");
}

/// The files the search's `end` records say rg saw binary data in, among
/// those it matched.
#[test]
fn stopped_files_are_read_from_end_records_of_matched_files() {
    let stdout = [
        r#"{"type":"end","data":{"path":{"text":"/ws/a.txt"},"binary_offset":7,"stats":{}}}"#,
        r#"{"type":"end","data":{"path":{"text":"/ws/t.txt"},"binary_offset":null,"stats":{}}}"#,
        r#"{"type":"end","data":{"path":{"text":"/ws/unmatched"},"binary_offset":3,"stats":{}}}"#,
        r#"{"type":"end","data":{"path":{"bytes":"L3dzL2L/"},"binary_offset":0,"stats":{}}}"#,
        r#"{"type":"end","data":{"path":{"text":"/ws/cut"#,
    ]
    .join("\n");
    let matched = ["/ws/a.txt", "/ws/t.txt", "/ws/cut"]
        .map(std::path::PathBuf::from)
        .into_iter()
        .collect();
    assert_eq!(stopped_files(&stdout, &matched), ["/ws/a.txt"]);
}

#[test]
fn the_stopped_notice_is_bounded() {
    assert_eq!(stopped_notice(&[]), None);
    let names: Vec<String> = (0..5).map(|i| format!("f{i}")).collect();
    assert_eq!(
        stopped_notice(&names).as_deref(),
        Some(
            r#"rg stopped reading 5 files at a NUL byte, after their first matches: "f0", "f1", "f2" (and 2 more). Name one as path to search it whole"#
        )
    );
}

/// The description says what a search skips and how long lines are shown.
#[test]
fn the_description_says_how_long_lines_and_binary_files_are_handled() {
    let (tool, _tmp) = grep_in(&[]);
    let description = tool.definition().description;
    assert!(
        description.contains(&format!(
            "A line over {MAX_LINE_BYTES} bytes is shown around its matches"
        )),
        "{description}"
    );
    assert!(
        description.contains("A directory search skips binary files (a NUL byte): when it finds nothing, a note names those holding a match; name one as path to search it. It stops reading a file at a NUL byte after a match (content output notes it; count output leaves the file out)"),
        "{description}"
    );
}
