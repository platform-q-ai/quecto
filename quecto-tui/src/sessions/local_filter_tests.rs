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
    let found = filter_listed(listed, query, SessionListScope::Global);
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

fn local_keys(listed: &[ResumeSessionSummary], query: &str) -> Vec<String> {
    let found = filter_listed(listed, query, SessionListScope::Local);
    found.into_iter().map(|row| row.key).collect()
}

/// R2-T8: in Local Folder only what lies below the rows' common folder is
/// matched — the repository's own name no longer names every local row.
#[test]
fn local_folder_matches_the_path_below_the_common_folder_only() {
    let listed = [
        row("cli:root", "plan", Some("/home/u/walrus")),
        row("cli:app", "notes", Some("/home/u/walrus/app")),
        row("cli:deep", "walrus title", Some("/home/u/walrus/docs/deep")),
        row("cli:old", "legacy", None),
    ];
    assert_eq!(local_keys(&listed, "walrus"), ["cli:deep"], "title only");
    assert_eq!(local_keys(&listed, "home"), [] as [&str; 0]);
    assert_eq!(local_keys(&listed, "app"), ["cli:app"]);
    assert_eq!(local_keys(&listed, "docs deep"), ["cli:deep"]);
    assert_eq!(keys(&listed, "walrus").len(), 3, "All Folders: whole path");
    // R3-T6: every row in ONE folder — it cannot be told from the group
    // root, and nothing would lie below it: the whole path is matched
    // (over-matching, never hiding a row the harness would find).
    let single = [row("cli:a", "plan", Some("/work/app"))];
    assert_eq!(local_keys(&single, "app"), ["cli:a"]);
    assert_eq!(local_keys(&single, "plan"), ["cli:a"]);
    let same = [
        row("cli:a", "plan", Some("/r/repo/app")),
        row("cli:b", "notes", Some("/r/repo/app")),
        row("cli:old", "legacy", None),
    ];
    assert_eq!(local_keys(&same, "app"), ["cli:a", "cli:b"]);
    assert_eq!(local_keys(&same, "repo notes"), ["cli:b"]);
    assert!(local_keys(&same, "elsewhere").is_empty());
    // A worktree beside the repository: their parent is the common folder.
    let beside = [
        row("cli:a", "plan", Some("/work/walrus")),
        row("cli:wt", "plan", Some("/work/walrus-wt")),
    ];
    assert_eq!(local_keys(&beside, "walrus-wt"), ["cli:wt"]);
    assert!(local_keys(&beside, "work").is_empty());
}
