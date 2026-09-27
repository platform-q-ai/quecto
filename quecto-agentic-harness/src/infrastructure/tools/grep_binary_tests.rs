//! #2202: binary files a directory search skips are not skipped silently:
//! the result says how many hold a match, and names the first few.
use super::grep_binary::{Checked, binary_files, notice};
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
            "1 binary file holds a match but was skipped: bin/ls. Name it as path to search it"
        ),
        "{out}"
    );
}

#[tokio::test]
async fn matches_found_still_say_which_binary_files_were_skipped() {
    let (tool, _tmp) = grep_in(&[
        ("a.bin", BINARY),
        ("b.bin", BINARY),
        ("notes.txt", b"GLIBC is the C library\n"),
    ]);
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(out.contains("notes.txt:1: GLIBC is the C library"), "{out}");
    assert!(
        out.contains("2 binary files hold matches but were skipped: a.bin, b.bin."),
        "{out}"
    );
}

#[tokio::test]
async fn listings_say_so_too() {
    let (tool, _tmp) = grep_in(&[("x.bin", BINARY), ("y.txt", b"GLIBC\n")]);
    for output in ["files", "count"] {
        let out = search(
            &tool,
            serde_json::json!({"pattern": "GLIBC", "output": output}),
        )
        .await;
        assert!(out.contains("y.txt"), "{output}: {out}");
        assert!(
            out.contains("1 binary file holds a match but was skipped: x.bin"),
            "{output}: {out}"
        );
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
    let out = search(&tool, serde_json::json!({"pattern": "hello"})).await;
    assert!(out.contains("y.txt:1: hello"), "{out}");
    assert!(!out.contains("binary"), "{out}");
}

/// The probe honours the search's own filters.
#[tokio::test]
async fn skipped_binary_files_are_counted_within_the_search_filters() {
    let (tool, _tmp) = grep_in(&[("x.bin", BINARY), ("x.dat", BINARY)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "GLIBC", "glob": "*.dat"}),
    )
    .await;
    assert!(out.contains("skipped: x.dat."), "{out}");
    assert!(!out.contains("x.bin"), "{out}");
}

/// The note stays bounded however many binary files match.
#[tokio::test]
async fn many_skipped_binary_files_are_counted_and_a_few_named() {
    let names: Vec<String> = (0..12).map(|i| format!("f{i:02}.bin")).collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), BINARY)).collect();
    let (tool, _tmp) = grep_in(&files);
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(
        out.contains("12 binary files hold matches but were skipped: f00.bin, f01.bin, f02.bin (and 9 more). Name one as path to search it"),
        "{out}"
    );
}

/// The files whose `end` record says rg saw binary data.
#[test]
fn binary_files_are_read_from_end_records() {
    let stdout = [
        r#"{"type":"begin","data":{"path":{"text":"/ws/a.bin"}}}"#,
        r#"{"type":"match","data":{"path":{"text":"/ws/a.bin"},"lines":{"text":"x\n"},"line_number":1,"absolute_offset":0,"submatches":[]}}"#,
        r#"{"type":"end","data":{"path":{"text":"/ws/a.bin"},"binary_offset":7,"stats":{}}}"#,
        r#"{"type":"end","data":{"path":{"text":"/ws/t.txt"},"binary_offset":null,"stats":{}}}"#,
        r#"{"type":"end","data":{"path":{"bytes":"L3dzL2L/"},"binary_offset":0,"stats":{}}}"#,
        r#"{"type":"end","data":{"path":{"text":"/ws/cut"#,
        r#"{"type":"summary","data":{}}"#,
    ]
    .join("\n");
    assert_eq!(binary_files(&stdout), ["/ws/a.bin", "/ws/b\u{fffd}"]);
}

#[test]
fn the_notice_is_bounded_and_says_when_it_is_a_floor() {
    let names = |n: usize| (0..n).map(|i| format!("f{i}")).collect::<Vec<_>>();
    assert_eq!(
        notice(&Checked::Found {
            files: names(0),
            complete: true
        }),
        None
    );
    assert_eq!(
        notice(&Checked::Found {
            files: names(2),
            complete: false
        })
        .unwrap(),
        "At least 2 binary files hold matches but were skipped: f0, f1. Name one as path to search it"
    );
    assert_eq!(
        notice(&Checked::Found {
            files: names(0),
            complete: false
        })
        .unwrap(),
        "Binary files are skipped, and they could not all be checked for matches"
    );
    assert_eq!(
        notice(&Checked::TooSlow).unwrap(),
        "Binary files are skipped, and they could not all be checked for matches"
    );
    assert_eq!(notice(&Checked::NotNeeded), None);
}

/// A tool whose rg runs `check` (shell) for the binary check and the real
/// rg for the search itself, in a workspace holding `files`.
#[cfg(unix)]
fn with_check(files: &[(&str, &[u8])], check: &str) -> (GrepTool, TempDir) {
    let (_, tmp) = grep_in(files);
    let script = tmp.path().join("check-rg");
    crate::infrastructure::test_support::executable::write_executable(
        &script,
        format!("#!/bin/sh\ncase \" $* \" in *' --binary '*) {check};; esac\nexec rg \"$@\"\n"),
    );
    let tool = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()))),
        script.to_string_lossy().into_owned(),
    );
    (tool, tmp)
}

#[cfg(unix)]
/// Prints rg's `end` record for `a.bin` in the workspace (where the stand-in
/// rg is), saying rg saw binary data in it.
const PRINT_BINARY_END: &str = r#"printf '{"type":"end","data":{"path":{"text":"%s/a.bin"},"binary_offset":4,"stats":{}}}\n' "$(dirname "$0")""#;

const UNCHECKED: &str = "Binary files are skipped, and they could not all be checked for matches";

/// A probe that outlasts the search by its grace is abandoned, and the
/// search is answered all the same.
#[cfg(unix)]
#[tokio::test]
async fn a_slow_binary_check_does_not_hold_the_search() {
    let (tool, _tmp) = with_check(&[("t.txt", b"GLIBC\n")], "exec sleep 30");
    let started = std::time::Instant::now();
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert!(out.contains("t.txt:1: GLIBC"), "{out}");
    assert!(out.contains(UNCHECKED), "{out}");
}

/// A check that fails (here: stopped at the rg timeout having printed
/// nothing) found nothing, which is not "no binary file holds a match".
#[cfg(unix)]
#[tokio::test]
async fn a_failed_binary_check_does_not_claim_there_is_nothing_to_say() {
    let (tool, _tmp) = with_check(&[("t.txt", b"GLIBC\n")], "exec sleep 30");
    let tool = tool.with_rg_timeout(std::time::Duration::from_millis(500));
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(out.contains("t.txt:1: GLIBC"), "{out}");
    assert!(out.contains(UNCHECKED), "{out}");
}

/// A check rg did not finish (an exit status past its own 0..=2) found
/// at least what it printed: the count is a floor.
#[cfg(unix)]
#[tokio::test]
async fn a_check_rg_did_not_finish_counts_at_least_what_it_found() {
    let (tool, _tmp) = with_check(
        &[("t.txt", b"GLIBC\n")],
        &format!("{PRINT_BINARY_END}; exit 3"),
    );
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(
        out.contains("At least 1 binary file holds a match but was skipped: a.bin."),
        "{out}"
    );
}

/// A check whose output was cut at the read cap is a floor too, even when
/// rg itself exited cleanly (a descendant still writing).
#[cfg(unix)]
#[tokio::test]
async fn a_check_cut_short_counts_at_least_what_it_found() {
    let flood = "(head -c 6000000 /dev/zero | tr '\\0' x) & exit 0";
    let (tool, _tmp) = with_check(
        &[("t.txt", b"GLIBC\n")],
        &format!("{PRINT_BINARY_END}; {flood}"),
    );
    let out = search(&tool, serde_json::json!({"pattern": "GLIBC"})).await;
    assert!(
        out.contains("At least 1 binary file holds a match but was skipped: a.bin."),
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
        out.contains("1 binary file holds a match but was skipped: y.bin."),
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
        out.contains("1 binary file holds a match but was skipped: sub/x.bin."),
        "{out}"
    );
}

/// The check is the search with `--binary` and one match per file, JSON
/// whatever the output mode, and never the search's own per-file cap.
#[test]
fn the_check_reads_binary_files_one_match_each() {
    let request = super::grep_request::parse_request(&serde_json::json!({
        "pattern": "x", "output": "files", "maxPerFile": 7
    }))
    .unwrap();
    let args = |pass| {
        build_rg_command("rg", Path::new("/ws"), Path::new("/ws"), &request, pass)
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    let caps = |args: &[String]| {
        args.windows(2)
            .filter(|pair| pair[0] == "--max-count")
            .map(|pair| pair[1].clone())
            .collect::<Vec<_>>()
    };
    let check = args(Pass::BinaryCheck);
    assert!(check.contains(&"--binary".to_string()), "{check:?}");
    assert!(check.contains(&"--json".to_string()), "{check:?}");
    assert!(
        !check.contains(&"--files-with-matches".to_string()),
        "{check:?}"
    );
    assert_eq!(caps(&check), ["1"], "{check:?}");
    let search = args(Pass::Search);
    assert!(!search.contains(&"--binary".to_string()), "{search:?}");
    assert_eq!(caps(&search), ["7"], "{search:?}");
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
        description.contains("A directory search skips binary files; a note counts those holding a match (name one as path to search it)"),
        "{description}"
    );
}
