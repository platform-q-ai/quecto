//! #2283 (epic #2265): the harness has no Python anywhere, tests included.
//! No `.py` source sits under `src` or `tests`, and nothing starts a Python
//! interpreter: no string literal of a Rust source under `src` or `tests`
//! (production and test code alike, `build.rs` too), and no line of a
//! Gherkin feature (whose text the steps run) or of a shell script a test
//! runs, names a `python` program, bare or by path, by any version. Test code reaches SQLite through
//! `rusqlite` and processes through `sh` or Rust fixtures.
//!
//! A text that names Python as data, never run (a command a policy check
//! classifies, a transcript a projection reads), is listed with its file in
//! [`PYTHON_AS_DATA`], an allowlist: nothing else may name it.
use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};

/// `(file, text)`: the only texts under `src` and `tests` that may name a
/// Python program, each data a test classifies or reads and never runs.
/// The text is a string literal's whole value, or a feature file's whole
/// line (trimmed).
const PYTHON_AS_DATA: &[(&str, &str)] = &[
    // The command policy classifies interpreter commands; none is run.
    (
        "quecto-agentic-harness/src/infrastructure/security/denylist_tests.rs",
        "python -m pytest tests/test_reboot.py",
    ),
    (
        "quecto-agentic-harness/src/infrastructure/security/denylist_tests.rs",
        r#"python -c "print('rm -rf /')""#,
    ),
    (
        "quecto-agentic-harness/src/infrastructure/security/denylist_tests.rs",
        "python <<EOF\nprint('reboot')\nEOF",
    ),
    (
        "quecto-agentic-harness/src/infrastructure/security/sandbox_escape_tests.rs",
        r#"python -c "print('halt')""#,
    ),
    (
        "quecto-agentic-harness/tests/features/security.feature",
        r#"| python -c "print('rm -rf /')"        |"#,
    ),
    // The docs must not teach the interpreter the board once ran in, and
    // the starter container must not offer it.
    (
        "quecto-agentic-harness/tests/docs/swarm_docs.rs",
        "python3 -I",
    ),
    (
        "quecto-agentic-harness/tests/integration/docker_create_image_contract.rs",
        "python",
    ),
];

/// Whole files that are data naming Python, never run, with why.
const PYTHON_DATA_FILES: &[(&str, &str)] = &[
    (
        ".quecto/containers/standard/Containerfile",
        "this repository's own dev container keeps its interpreter (epic P6)",
    ),
    (
        "quecto-agentic-harness/tests/fixtures/claude_code/rt.stream.jsonl",
        "a recorded Claude Code transcript a projection test reads",
    ),
    (RULE_FILE, "this rule's own spelling tests and allowlists"),
];

/// This rule's file, whose spelling tests and allowlists name Python on
/// purpose.
const RULE_FILE: &str = "quecto-agentic-harness/tests/architecture/no_python.rs";

/// Every word of `text` that names a Python program: `python`, optionally
/// versioned (`python3`, `python3.12`), bare or by path.
pub(super) fn python_programs_in_text(text: &str) -> Vec<String> {
    static PROGRAM: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^python[0-9.]*$").expect("program regex"));
    if !text.contains("python") {
        return Vec::new();
    }
    let program = &*PROGRAM;
    // A word of a command holds only these (an allowlist): any other
    // character (a redirection, a brace list, an escape, a quote) ends it.
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-')))
        .filter(|word| program.is_match(word.rsplit('/').next().unwrap_or(word)))
        .map(str::to_owned)
        .collect()
}

/// The value of every string and byte-string literal in `tokens`, doc
/// comments (`#[doc = "…"]`, `#![doc = "…"]`) included: a doctest or an
/// example in one can start an interpreter too (#2344 review L1).
fn literals(tokens: TokenStream, found: &mut Vec<String>) {
    for tree in tokens {
        match tree {
            TokenTree::Group(group) => literals(group.stream(), found),
            TokenTree::Literal(literal) => match syn::parse_str::<syn::Lit>(&literal.to_string()) {
                Ok(syn::Lit::Str(text)) => found.push(text.value()),
                Ok(syn::Lit::ByteStr(bytes)) => {
                    found.push(String::from_utf8_lossy(&bytes.value()).into_owned());
                }
                Ok(syn::Lit::CStr(text)) => {
                    found.push(text.value().to_string_lossy().into_owned());
                }
                Ok(_) | Err(_) => {}
            },
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
    }
}

/// The texts of one file that name a Python program: a Rust source's
/// string literals and doc comments, or any other text file's lines.
fn python_texts(path: &str, source: &str) -> Vec<String> {
    let texts: Vec<String> = match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("rs") => {
            let tokens: TokenStream = source
                .parse()
                .unwrap_or_else(|error| panic!("{path} tokenizes: {error}"));
            let mut found = Vec::new();
            literals(tokens, &mut found);
            found
        }
        _ => source.lines().map(|line| line.trim().to_owned()).collect(),
    };
    texts
        .into_iter()
        .filter(|text| !python_programs_in_text(text).is_empty())
        .collect()
}

/// The workspace root: every crate, the repository's scripts, its CI and
/// container files (#2344 review L2).
const WORKSPACE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// The folders the scan never enters: build output and git's own.
fn skipped_dir(relative: &str) -> bool {
    matches!(relative, ".git" | "target") || relative.ends_with("/target")
}

/// Every file under the workspace's `dir` (relative to the workspace).
fn files_under(dir: &str, found: &mut Vec<String>) {
    let absolute = Path::new(WORKSPACE).join(dir);
    let entries =
        std::fs::read_dir(&absolute).unwrap_or_else(|error| panic!("read {dir}: {error}"));
    for entry in entries {
        let name = entry.expect("a directory entry").file_name();
        let name = name.to_string_lossy();
        let relative = match dir {
            "" => name.into_owned(),
            dir => format!("{dir}/{name}"),
        };
        match Path::new(WORKSPACE).join(&relative).is_dir() {
            true if skipped_dir(&relative) => {}
            true => files_under(&relative, found),
            false => found.push(relative),
        }
    }
}

/// Every file of the workspace, relative to its root, sorted.
fn harness_files() -> Vec<String> {
    let mut files = Vec::new();
    files_under("", &mut files);
    files.sort();
    files
}

/// Prose the scan does not read: Markdown (a command in it runs nowhere).
fn prose(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "md")
}

/// The files the Python scan reads: every workspace file but prose (a file
/// that is not UTF-8 text is passed over when read).
fn scanned(files: &[String]) -> Vec<&String> {
    files.iter().filter(|path| !prose(path)).collect()
}

/// A workspace file's text, or `None` for a file that is not UTF-8 (a
/// database, an image): no interpreter runs from one.
fn text_of(path: &str) -> Option<String> {
    let bytes = std::fs::read(Path::new(WORKSPACE).join(path))
        .unwrap_or_else(|error| panic!("read {path}: {error}"));
    String::from_utf8(bytes).ok()
}

#[test]
fn no_python_sources_in_the_harness() {
    let files = harness_files();
    assert!(files.len() > 1000, "the workspace's files are listed");
    let python: Vec<&String> = files
        .iter()
        .filter(|path| {
            Path::new(path)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
        })
        .collect();
    assert!(
        python.is_empty(),
        "Python sources in the workspace: {python:?}"
    );
    let production = super::teardown_authority::production_files();
    assert!(production.len() > 200, "the production files are listed");
    let named = format!("{:?}", "python3");
    for path in &production {
        let source = super::teardown_authority::production_source(path);
        assert!(
            !source.contains(&named),
            "{path} names {named} in production code"
        );
    }
}

#[test]
fn no_python_in_the_harness() {
    let files = harness_files();
    let scanned = scanned(&files);
    assert!(
        scanned.iter().any(|path| path.ends_with(".feature")) && scanned.len() > 1000,
        "the workspace's sources, features and scripts are scanned"
    );
    for (file, reason) in PYTHON_DATA_FILES {
        assert!(
            files.iter().any(|path| path == file),
            "{file} ({reason}) is gone"
        );
    }
    let mut named = Vec::new();
    let mut allowed_seen = Vec::new();
    for path in scanned {
        let Some(source) = text_of(path) else {
            continue;
        };
        if PYTHON_DATA_FILES.iter().any(|(file, _)| file == path) {
            continue;
        }
        for text in python_texts(path, &source) {
            match PYTHON_AS_DATA
                .iter()
                .position(|(file, data)| file == path && *data == text)
            {
                Some(entry) => allowed_seen.push(entry),
                None => named.push(format!("{path}: {text:?}")),
            }
        }
    }
    assert!(
        named.is_empty(),
        "the workspace starts no Python; these texts name an interpreter:\n{}",
        named.join("\n")
    );
    // Every allowlisted text is still where it is listed: a stale entry
    // would let a later text through.
    for (entry, (file, data)) in PYTHON_AS_DATA.iter().enumerate() {
        assert!(
            allowed_seen.contains(&entry),
            "{file}: the allowlisted {data:?} is not there any more"
        );
    }
}

/// #2282 review N3, #2283: the scan catches every spelling a program can
/// start Python by, in a Rust literal of any kind or a feature's step, and
/// nothing that merely begins with the word or names it in prose.
#[test]
fn the_python_scan_catches_every_interpreter_spelling() {
    for caught in [
        r#"Command::new("python3")"#,
        r#"Command::new("python")"#,
        r#"Command::new("/usr/bin/python3")"#,
        r#"Command::new("python3.12")"#,
        r#"Command::new("/usr/bin/env").arg("python3")"#,
        r#"Command::new("sh").args(["-c", "/usr/bin/env python -c 'x'"])"#,
        r#"const PROGRAM: &str = "python2";"#,
        r##"Command::new(r#"python3"#)"##,
        "#!/usr/bin/python3\nprint(1)",
        "exec python3 - \"$sock\" <<'PY'",
    ] {
        assert!(
            !python_programs_in_text(caught).is_empty(),
            "the scan misses {caught}"
        );
    }
    for clean in [
        r#"Command::new("bash")"#,
        "pythonic",
        "cpython",
        "python_lab",
        "no Python workbench",
        "Python's json.dumps",
        "tests/swarm_policy_test.py",
    ] {
        assert_eq!(
            python_programs_in_text(clean),
            Vec::<String>::new(),
            "a false positive in {clean}"
        );
    }
    // A Rust source's literals are read, raw and byte strings included,
    // and its doc comments; its plain comments are not.
    let word = "python3";
    let source = format!(
        "//! Once the {word} board ran here.\n\
         /// {word} prose.\n\
         fn f() {{\n\
             // {word} in a comment\n\
             let _ = \"{word}\";\n\
             let _ = r#\"{word}\"#;\n\
             let _ = b\"{word}\";\n\
         }}\n"
    );
    // Two doc comments and three literals.
    assert_eq!(python_texts("src/a.rs", &source).len(), 5);
    assert_eq!(
        python_texts("tests/a.rs", "fn f() { let _ = 1; }"),
        Vec::<String>::new()
    );
    // A feature's steps are its text.
    let step = format!("When the agent runs \"{word} -c 'print(1)'\"");
    let feature = format!("Feature: x\n  Scenario: y\n    {step}\n");
    assert_eq!(python_texts("tests/features/a.feature", &feature), [step]);
}

/// #2283 review L1: the scan splits a text at every character a word of a
/// command cannot hold, so a redirection, a brace list or an escape cannot
/// hide the interpreter's name.
#[test]
fn the_python_scan_splits_at_every_shell_delimiter() {
    let word = "python3";
    for caught in [
        format!("cat x >{word}"),
        format!("{word}<<EOF"),
        format!("{word}<input"),
        format!("{{{word},sh}} -c x"),
        format!("\\{word} -c x"),
        format!("env X=1 {word}"),
        format!("[{word}]"),
    ] {
        assert!(
            !python_programs_in_text(&caught).is_empty(),
            "the scan misses {caught}"
        );
    }
}

/// #2283 review L1: doc comments are read too (a doctest or an example in
/// one can start an interpreter).
#[test]
fn the_python_scan_reads_doc_comments() {
    let word = "python3";
    let source = format!("/// Run `{word} -c x` first.\nfn f() {{}}\n");
    assert_eq!(python_texts("src/a.rs", &source).len(), 1);
}

/// #2283 review L1/L2: the scan covers the whole workspace: every crate's
/// sources, tests and examples, and the repository's scripts.
#[test]
fn the_python_scan_covers_the_whole_workspace() {
    let files = harness_files();
    let scanned = scanned(&files);
    for expected in [
        "quecto-tui/src/shell/child_watch_tests.rs",
        "quecto-tui/tests/bdd/tui_owner_signals_steps.rs",
        "quecto-agentic-harness/examples/board_op_overhead.rs",
        "scripts/run-bdd-shards.sh",
        "scripts/check-pr-review-threads-resolved.sh",
        ".github/workflows/ci.yml",
    ] {
        assert!(
            scanned.iter().any(|path| path.as_str() == expected),
            "the scan misses {expected}"
        );
    }
}
