//! Differential test of the Python-compatible JSON codec (#2268): every
//! corpus value is written by `python3` and by the Rust codec, in both
//! styles, and the texts must be byte-identical.

use quecto::infrastructure::persistence::swarm_board::py_json;
use serde_json::Value;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/json_corpus.json"
);

/// Prints, per corpus value, its `encode` text then its plain `dumps` text,
/// one per line (`ensure_ascii` keeps every text on one line).
const PYTHON_WRITER: &str = "import json, sys
corpus = json.load(open(sys.argv[1], encoding='utf-8'))
for value in corpus['values']:
    print(json.dumps(value, sort_keys=True, separators=(',', ':')))
    print(json.dumps(value))
";

fn corpus_values() -> Vec<Value> {
    let text = std::fs::read_to_string(CORPUS).expect("corpus fixture is readable");
    let corpus: Value = serde_json::from_str(&text).expect("corpus fixture is JSON");
    corpus["values"]
        .as_array()
        .expect("corpus has a values list")
        .clone()
}

fn python_texts() -> Vec<String> {
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
    String::from_utf8(output.stdout)
        .expect("ensure_ascii output is ASCII")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn encode_and_dumps_are_byte_identical_to_python_for_a_corpus() {
    let values = corpus_values();
    let texts = python_texts();
    assert!(values.len() >= 200, "corpus holds about 200 values");
    assert_eq!(texts.len(), values.len() * 2, "two Python texts per value");

    let mut mismatches = Vec::new();
    for (index, (value, pair)) in values.iter().zip(texts.chunks(2)).enumerate() {
        let encoded = py_json::encode(value);
        if encoded != pair[0] {
            mismatches.push(format!(
                "#{index} encode\n  rust:   {encoded}\n  python: {}",
                pair[0]
            ));
        }
        let dumped = py_json::dumps(value);
        if dumped != pair[1] {
            mismatches.push(format!(
                "#{index} dumps\n  rust:   {dumped}\n  python: {}",
                pair[1]
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatch(es):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
