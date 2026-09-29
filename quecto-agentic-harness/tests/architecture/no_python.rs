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
        "src/infrastructure/security/denylist_tests.rs",
        "python -m pytest tests/test_reboot.py",
    ),
    (
        "src/infrastructure/security/denylist_tests.rs",
        r#"python -c "print('rm -rf /')""#,
    ),
    (
        "src/infrastructure/security/denylist_tests.rs",
        "python <<EOF\nprint('reboot')\nEOF",
    ),
    (
        "src/infrastructure/security/sandbox_escape_tests.rs",
        r#"python -c "print('halt')""#,
    ),
    (
        "tests/features/security.feature",
        r#"| python -c "print('rm -rf /')"        |"#,
    ),
    // The docs must not teach the interpreter the board once ran in, and
    // the starter container must not offer it.
    ("tests/docs/swarm_docs.rs", "python3 -I"),
    (
        "tests/integration/docker_create_image_contract.rs",
        "python",
    ),
    // This rule's own spelling tests.
    (RULE_FILE, r#"Command::new("python3")"#),
    (RULE_FILE, r#"Command::new("python")"#),
    (RULE_FILE, r#"Command::new("/usr/bin/python3")"#),
    (RULE_FILE, r#"Command::new("python3.12")"#),
    (RULE_FILE, r#"Command::new("/usr/bin/env").arg("python3")"#),
    (
        RULE_FILE,
        r#"Command::new("sh").args(["-c", "/usr/bin/env python -c 'x'"])"#,
    ),
    (RULE_FILE, r#"const PROGRAM: &str = "python2";"#),
    (RULE_FILE, r##"Command::new(r#"python3"#)"##),
    (RULE_FILE, "#!/usr/bin/python3\nprint(1)"),
    (RULE_FILE, "exec python3 - \"$sock\" <<'PY'"),
    (RULE_FILE, "python3"),
];

/// This rule's file, whose spelling tests name Python on purpose, and
/// which holds [`PYTHON_AS_DATA`]'s texts: a text of this file is allowed
/// when it is any entry's text.
const RULE_FILE: &str = "tests/architecture/no_python.rs";

/// Every word of `text` that names a Python program: `python`, optionally
/// versioned (`python3`, `python3.12`), bare or by path.
pub(super) fn python_programs_in_text(text: &str) -> Vec<String> {
    let program = regex::Regex::new(r"^python[0-9.]*$").expect("program regex");
    text.split(|c: char| {
        c.is_whitespace() || matches!(c, '\'' | '"' | ';' | '&' | '|' | '(' | ')' | '`' | '=')
    })
    .filter(|word| program.is_match(word.rsplit('/').next().unwrap_or(word)))
    .map(str::to_owned)
    .collect()
}

/// The value of every string and byte-string literal in `tokens`, doc
/// comments (`#[doc = "…"]`, `#![doc = "…"]`) left out: prose may name
/// Python, code may not.
fn literals(tokens: TokenStream, found: &mut Vec<String>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut index = 0;
    while index < trees.len() {
        match &trees[index] {
            TokenTree::Punct(punct) if punct.as_char() == '#' => {
                let attribute = match trees.get(index + 1) {
                    Some(TokenTree::Punct(bang)) if bang.as_char() == '!' => index + 2,
                    _ => index + 1,
                };
                if let Some(TokenTree::Group(group)) = trees.get(attribute)
                    && is_doc(group.stream())
                {
                    index = attribute + 1;
                    continue;
                }
            }
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
        index += 1;
    }
}

fn is_doc(attribute: TokenStream) -> bool {
    matches!(attribute.into_iter().next(), Some(TokenTree::Ident(ident)) if ident == "doc")
}

/// The texts of one file that name a Python program: a Rust source's
/// string literals, or a feature file's lines.
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
        Some("feature" | "sh") => source.lines().map(|line| line.trim().to_owned()).collect(),
        other => panic!(
            "{path}: only Rust sources, features and shell scripts are scanned, not {other:?}"
        ),
    };
    texts
        .into_iter()
        .filter(|text| !python_programs_in_text(text).is_empty())
        .collect()
}

/// Every file under `dir`, relative to the crate.
fn files_under(dir: &str, found: &mut Vec<String>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|error| panic!("read {dir}: {error}"));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        let path = path.to_string_lossy().into_owned();
        if Path::new(&path).is_dir() {
            files_under(&path, found);
        } else {
            found.push(path);
        }
    }
}

fn harness_files() -> Vec<String> {
    let mut files = Vec::new();
    files_under("src", &mut files);
    files_under("tests", &mut files);
    if Path::new("build.rs").exists() {
        files.push("build.rs".to_owned());
    }
    files.sort();
    files
}

/// The files the Python scan reads: Rust sources, Gherkin features and
/// shell scripts (fixtures a test runs).
fn scanned(files: &[String]) -> Vec<&String> {
    files
        .iter()
        .filter(|path| {
            Path::new(path)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| matches!(extension, "rs" | "feature" | "sh"))
        })
        .collect()
}

#[test]
fn no_python_sources_in_the_harness() {
    let files = harness_files();
    assert!(files.len() > 500, "the harness's files are listed");
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
        "Python sources in the harness: {python:?}"
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
        scanned.iter().any(|path| path.ends_with(".feature")) && scanned.len() > 500,
        "the harness's Rust sources and features are scanned"
    );
    let mut named = Vec::new();
    let mut allowed_seen = Vec::new();
    for path in scanned {
        let source =
            std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        for text in python_texts(path, &source) {
            let listed = PYTHON_AS_DATA
                .iter()
                .position(|(file, data)| file == path && *data == text);
            let rule_text =
                path == RULE_FILE && PYTHON_AS_DATA.iter().any(|(_, data)| *data == text);
            match (listed, rule_text) {
                (Some(entry), _) => allowed_seen.push(entry),
                (None, true) => {}
                (None, false) => named.push(format!("{path}: {text:?}")),
            }
        }
    }
    assert!(
        named.is_empty(),
        "the harness starts no Python; these texts name an interpreter:\n{}",
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
    // A Rust source's literals are read, raw and byte strings included;
    // its doc comments and plain comments are not.
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
    assert_eq!(python_texts("src/a.rs", &source), [word, word, word]);
    assert_eq!(
        python_texts("tests/a.rs", "fn f() { let _ = 1; }"),
        Vec::<String>::new()
    );
    // A feature's steps are its text.
    let step = format!("When the agent runs \"{word} -c 'print(1)'\"");
    let feature = format!("Feature: x\n  Scenario: y\n    {step}\n");
    assert_eq!(python_texts("tests/features/a.feature", &feature), [step]);
}
