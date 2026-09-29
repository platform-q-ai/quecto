//! Golden test of the Python-compatible JSON codec (#2268, #2283): every
//! corpus value, and the value of every corpus text, is written by the Rust
//! codec in both styles, and the texts must be byte-identical to what
//! CPython (>= 3.13, whose messages the codec reproduces; 3.12 and earlier
//! word trailing-comma errors differently) wrote; every invalid text must
//! be refused with Python's message (for a syntax error: its text, line,
//! column and char). Python's texts are frozen in
//! `json_corpus.golden.json`: until #2283 `python3` wrote them on every run.

use quecto::infrastructure::persistence::swarm_board::py_json::{self, PyJson, PyStr};

pub(crate) const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/json_corpus.json"
);
/// What Python wrote for [`CORPUS`]: `lines`, per corpus value and then per
/// text's value, its `encode` text and its plain `dumps` text, then per
/// invalid text `refused: ` and Python's message.
pub(crate) const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/json_corpus.golden.json"
);

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

/// Python's output lines, as frozen.
pub(crate) fn golden_lines() -> Vec<String> {
    let golden: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(GOLDEN).expect("the golden corpus is readable"),
    )
    .expect("the golden corpus is JSON");
    golden["lines"]
        .as_array()
        .expect("the golden corpus holds its lines")
        .iter()
        .map(|line| line.as_str().expect("a golden line is text").to_owned())
        .collect()
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
    let lines = golden_lines();
    assert!(values.len() >= 200, "corpus holds about 200 values");
    let written = values.len() + decoded.len();
    assert_eq!(
        lines.len(),
        written * 2 + invalid.len(),
        "one line per golden output"
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
                "#{index} encode\n  rust:   {encoded}\n  golden: {}",
                pair[0]
            ));
        }
        let dumped = rust_line(py_json::dumps(value));
        if dumped != pair[1] {
            mismatches.push(format!(
                "#{index} dumps\n  rust:   {dumped}\n  golden: {}",
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
        if rust != *verdict || !verdict.starts_with("refused: ") {
            mismatches.push(format!("invalid {text:?}: rust {rust}, golden {verdict}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatch(es):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
