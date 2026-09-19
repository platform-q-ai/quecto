use super::*;

fn row(key: &str, title: &str, dir: Option<&str>) -> ResumeSessionSummary {
    ResumeSessionSummary {
        key: key.into(),
        title: title.into(),
        message_count: 1,
        updated_unix_secs: None,
        execution_dir: dir.map(str::to_string),
        resume_eligible: true,
        home_version: None,
        unscoped: false,
        repository_label: Some("never-matched".into()),
        matched: Vec::new(),
    }
}

fn keys(listed: &[ResumeSessionSummary], query: &str) -> Vec<String> {
    let found = filter_listed(listed, query);
    found.into_iter().map(|row| row.key).collect()
}

#[test]
fn every_word_must_occur_in_the_title_or_folder_and_a_key_matches_whole() {
    let listed = [
        row("cli:a", "Fix the ΟΔΟΣ renderer", Some("/work/app")),
        row("cli:b", "İstanbul notes", Some("/work/Straße/docs")),
        row("cli:c", "de\u{202e}pl\u{200b}oy", None),
    ];
    assert_eq!(keys(&listed, "renderer  FIX"), ["cli:a"]);
    assert_eq!(keys(&listed, "οδος"), ["cli:a"]);
    assert_eq!(keys(&listed, "istanbul strasse"), ["cli:b"]);
    assert_eq!(keys(&listed, "deploy"), ["cli:c"]);
    assert_eq!(
        keys(&listed, "work"),
        ["cli:a", "cli:b"],
        "listing order kept"
    );
    assert_eq!(keys(&listed, " cli:b "), ["cli:b"]);
    assert!(keys(&listed, "cli:").is_empty(), "never a key fragment");
    assert!(keys(&listed, "CLI:B").is_empty(), "never a folded key");
    assert!(
        keys(&listed, "never-matched").is_empty(),
        "no repository label"
    );
    assert!(
        keys(&listed, "fix istanbul").is_empty(),
        "words are ANDed per row"
    );
    assert_eq!(
        keys(&listed, " \u{200b} ").len(),
        3,
        "nothing visible: everything"
    );
    assert_eq!(keys(&listed, ".*").len(), 0, "literal, never a pattern");
}
