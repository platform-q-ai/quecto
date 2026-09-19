use super::*;

fn skip(file: &str, why: &str) -> (String, String) {
    (file.to_string(), why.to_string())
}

#[test]
fn a_skipped_record_is_named_once_by_the_catalogues_line_else_by_the_walks() {
    let mut lines = vec![
        "cli_a.json: empty session".to_string(),
        "odd: name.json: record has no key".to_string(),
        "index rebuilt; cli_c.json: mentioned, not named".to_string(),
    ];
    let skipped = vec![
        skip("cli_a.json", "walk's words"),
        skip("odd: name.json", "walk's words"),
        skip("odd", "its own file"),
        skip("cli_c.json", "unreadable"),
        skip("cli_c.json", "listed twice"),
        skip("cli_d.json", "why: with: colons"),
        skip(
            "cli_d.json: session record not listed",
            "a name that is a line's prefix",
        ),
    ];
    name_skipped(&mut lines, skipped);
    assert_eq!(
        lines[3..],
        [
            "cli_c.json: session record not listed: unreadable",
            "cli_d.json: session record not listed: why: with: colons",
        ]
    );
    assert_eq!(lines.len(), 5);
}

/// R3-H1: 40,000 corrupt records are in BOTH lists; naming them is one pass,
/// not one scan of every line per file (the bound is generous: the quadratic
/// form took far longer than this in a debug build).
#[test]
fn naming_is_linear_in_the_number_of_bad_records() {
    let n = 40_000;
    let file = |i: usize| format!("chat-bad{i:05}.json");
    let mut lines: Vec<String> = (0..n)
        .map(|i| format!("{}: EOF while parsing", file(i)))
        .collect();
    let skipped: Vec<_> = (0..2 * n).map(|i| skip(&file(i), "EOF")).collect();
    let started = std::time::Instant::now();
    name_skipped(&mut lines, skipped);
    let took = started.elapsed();
    assert_eq!(lines.len(), 2 * n);
    let names: std::collections::HashSet<&str> = lines
        .iter()
        .map(|line| line.split_once(": ").unwrap().0)
        .collect();
    assert_eq!(names.len(), 2 * n, "every file named exactly once");
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
}
