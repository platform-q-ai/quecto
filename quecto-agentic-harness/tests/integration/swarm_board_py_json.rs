//! Differential test of the Python-compatible JSON codec (#2268): every
//! corpus value, and the value of every corpus text, is written by
//! `python3` and by the Rust codec in both styles, and the texts must be
//! byte-identical; every invalid text must be refused by both, with the same
//! message (for a syntax error: Python's text, line, column and char) when the
//! Python running the test is CPython >= 3.13, whose messages the codec
//! reproduces (3.12 and earlier word trailing-comma errors differently).

use quecto::infrastructure::persistence::swarm_board::py_json::{self, PyJson, PyStr};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/json_corpus.json"
);

/// Prints whether this Python's error text is the one the codec reproduces
/// (`exact-errors: True` from 3.13), then, per corpus value and then per text's value, its `encode` text
/// and its plain `dumps` text, one per line (`ensure_ascii` keeps every
/// text on one line), then per invalid text `refused: ` and Python's
/// message, or `accepted`.
const PYTHON_WRITER: &str = "import json, sys
print('exact-errors: ' + str(sys.version_info >= (3, 13)))
corpus = json.load(open(sys.argv[1], encoding='utf-8'))
values = corpus['values'] + [json.loads(text) for text in corpus['texts']]
for value in values:
    print(json.dumps(value, sort_keys=True, separators=(',', ':')))
    print(json.dumps(value))
for text in corpus['invalid']:
    try:
        json.loads(text)
        print('accepted')
    except (ValueError, RecursionError) as error:
        print('refused: ' + str(error))
";

/// The corpus, read by the codec under test.
fn corpus() -> PyJson {
    let text = std::fs::read_to_string(CORPUS).expect("corpus fixture is readable");
    py_json::decode(&text).expect("corpus fixture is JSON")
}

fn section<'a>(corpus: &'a PyJson, name: &str) -> &'a [PyJson] {
    match corpus {
        PyJson::Object(object) => match object.get(&PyStr::from(name)) {
            Some(PyJson::List(items)) => items,
            other => panic!("corpus {name} is a list, got {other:?}"),
        },
        other => panic!("corpus is an object, got {other:?}"),
    }
}

fn text_of(item: &PyJson) -> &str {
    match item {
        PyJson::Str(text) => text.as_str().expect("corpus texts are valid UTF-8"),
        other => panic!("corpus texts are strings, got {other:?}"),
    }
}

/// Python's output lines, and whether its error text is comparable.
fn python_lines() -> (bool, Vec<String>) {
    let output = std::process::Command::new("python3")
        .args(["-I", "-c", PYTHON_WRITER, CORPUS])
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("python3 is required for the differential test");
    assert!(
        output.status.success(),
        "python3 failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut lines: Vec<String> = String::from_utf8(output.stdout)
        .expect("ensure_ascii output is ASCII")
        .lines()
        .map(str::to_owned)
        .collect();
    assert!(!lines.is_empty(), "python3 printed its version verdict");
    let exact_errors = match lines.remove(0).as_str() {
        "exact-errors: True" => true,
        "exact-errors: False" => false,
        other => panic!("unexpected first line from python3: {other}"),
    };
    (exact_errors, lines)
}

fn rust_line(written: Result<String, py_json::PyJsonError>) -> String {
    written.unwrap_or_else(|error| format!("<error: {error}>"))
}

#[test]
fn encode_and_dumps_are_byte_identical_to_python_for_a_corpus() {
    let corpus = corpus();
    let values = section(&corpus, "values");
    let texts = section(&corpus, "texts");
    let invalid = section(&corpus, "invalid");
    let decoded: Vec<PyJson> = texts
        .iter()
        .map(|text| py_json::decode(text_of(text)).expect("Python reads every corpus text"))
        .collect();
    let (exact_errors, lines) = python_lines();
    assert!(values.len() >= 200, "corpus holds about 200 values");
    let written = values.len() + decoded.len();
    assert_eq!(
        lines.len(),
        written * 2 + invalid.len(),
        "one line per Python output"
    );

    let mut mismatches = Vec::new();
    for (index, (value, pair)) in values
        .iter()
        .chain(&decoded)
        .zip(lines.chunks(2))
        .enumerate()
    {
        let encoded = rust_line(py_json::encode(value));
        if encoded != pair[0] {
            mismatches.push(format!(
                "#{index} encode\n  rust:   {encoded}\n  python: {}",
                pair[0]
            ));
        }
        let dumped = rust_line(py_json::dumps(value));
        if dumped != pair[1] {
            mismatches.push(format!(
                "#{index} dumps\n  rust:   {dumped}\n  python: {}",
                pair[1]
            ));
        }
    }
    for (text, verdict) in invalid.iter().zip(&lines[written * 2..]) {
        let text = text_of(text);
        let rust = match py_json::decode(text) {
            Ok(_) => "accepted".to_owned(),
            Err(error) => format!("refused: {error}"),
        };
        // Before 3.13 only the verdict is comparable (see the module docs).
        let same = if exact_errors {
            rust == *verdict
        } else {
            rust.starts_with("refused: ")
        };
        if !same || !verdict.starts_with("refused: ") {
            mismatches.push(format!("invalid {text:?}: rust {rust}, python {verdict}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatch(es):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
