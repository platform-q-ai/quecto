use super::*;
use crate::domain::session_home::{AssociationProvenance, SessionHome, WorkspaceGroup};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

fn git_home(dir: &str, common: &str) -> SessionHomeScope {
    SessionHomeScope::Scoped(SessionHome {
        execution_dir: PathBuf::from(dir),
        group: WorkspaceGroup::Git {
            common_dir: PathBuf::from(common),
        },
        provenance: AssociationProvenance::SavedHere,
    })
}

fn folder_home(dir: impl Into<PathBuf>) -> SessionHomeScope {
    let directory = dir.into();
    SessionHomeScope::Scoped(SessionHome {
        execution_dir: directory.clone(),
        group: WorkspaceGroup::Folder { directory },
        provenance: AssociationProvenance::SavedHere,
    })
}

fn matched(
    query: &str,
    key: &str,
    title: &str,
    home: &SessionHomeScope,
) -> Option<Vec<MatchedField>> {
    let fields = SessionMetadataFields {
        key,
        title,
        home,
        local_root: None,
    };
    MetadataQuery::parse(query)
        .expect("searchable")
        .matches(&fields)
}

#[test]
fn each_field_is_matched_on_its_own_and_reported_in_rank_order() {
    let home = git_home("/work/alpha/src", "/work/reponame/.git");
    assert_eq!(
        matched("flaky", "chat-1-a", "Fix the FLAKY test", &home),
        Some(vec![MatchedField::Title])
    );
    assert_eq!(
        matched("chat-1-a", "chat-1-a", "t", &home),
        Some(vec![MatchedField::Key])
    );
    assert_eq!(
        matched("reponame", "chat-1-a", "t", &home),
        Some(vec![MatchedField::Repository])
    );
    assert_eq!(
        matched("alpha/src", "chat-1-a", "t", &home),
        Some(vec![MatchedField::Path])
    );
    assert_eq!(matched("nothing-like-it", "chat-1-a", "t", &home), None);
    // One term in the title and one in the path: both fields, title first.
    assert_eq!(
        matched("alpha flaky", "k", "flaky", &home),
        Some(vec![MatchedField::Title, MatchedField::Path])
    );
    // Every term must be found somewhere.
    assert_eq!(matched("flaky absent", "k", "flaky", &home), None);
}

#[test]
fn a_key_is_matched_whole_and_byte_exact_never_as_a_fragment_or_a_fold() {
    let home = SessionHomeScope::LegacyUnscoped;
    assert_eq!(
        matched("  cli:Mine ", "cli:Mine", "t", &home),
        Some(vec![MatchedField::Key])
    );
    for near in [
        "cli:mine",
        "cli:Min",
        "li:Mine",
        "cli:Mine2",
        "cli:\u{200b}Mine",
    ] {
        assert_eq!(matched(near, "cli:Mine", "t", &home), None, "{near:?}");
    }
    // A key that also occurs in the title matches both.
    assert_eq!(
        matched("cli:Mine", "cli:Mine", "about cli:mine", &home),
        Some(vec![MatchedField::Key, MatchedField::Title])
    );
}

#[test]
fn legacy_and_unavailable_homes_are_matched_by_title_and_key_only() {
    for home in [
        SessionHomeScope::LegacyUnscoped,
        SessionHomeScope::Unavailable("/work/secret".into()),
    ] {
        assert_eq!(
            matched("old", "cli:old", "An OLD chat", &home),
            Some(vec![MatchedField::Title])
        );
        assert_eq!(
            matched("secret", "cli:old", "An old chat", &home),
            None,
            "a reason is no path"
        );
        assert_eq!(repository_label(&home), None);
        assert_eq!(execution_path(&home), None);
    }
}

#[test]
fn metacharacters_are_literal_text() {
    let home = folder_home("/work/a.b");
    assert_eq!(matched(".*", "k", "anything at all", &home), None);
    assert_eq!(matched("a.b", "k", "t", &folder_home("/work/aXb")), None);
    assert_eq!(
        matched("[x]*?", "k", "why [X]*? indeed", &home),
        Some(vec![MatchedField::Title])
    );
    assert_eq!(
        matched("\\d+$", "k", "cost \\D+$", &home),
        Some(vec![MatchedField::Title])
    );
    assert_eq!(
        matched("%_", "k", "100%_done", &home),
        Some(vec![MatchedField::Title])
    );
}

#[test]
fn invisible_and_control_characters_neither_hide_nor_forge_a_match() {
    let home = folder_home("/work/pro\u{200b}ject");
    // Hidden characters in the metadata do not hide the record…
    assert_eq!(
        matched("project", "k", "t", &home),
        Some(vec![MatchedField::Repository, MatchedField::Path])
    );
    // An escape byte is dropped; its printable tail is ordinary literal text.
    let escaped = "de\u{202e}pl\u{ad}oy\u{1b}[31m";
    assert_eq!(visible_text(escaped), "deploy[31m");
    assert_eq!(
        matched("oy[31m", "k", escaped, &home),
        Some(vec![MatchedField::Title])
    );
    assert_eq!(
        matched("deploy", "k", "de\u{202e}pl\u{ad}oy", &home),
        Some(vec![MatchedField::Title])
    );
    // …and hidden characters in the query are not searched for.
    assert_eq!(
        matched("pro\u{feff}je\u{0}ct", "k", "t", &home),
        Some(vec![MatchedField::Repository, MatchedField::Path])
    );
    // Nothing visible at all names every session, with no field.
    let everything = MetadataQuery::parse(" \u{200b}\u{202e}\t").unwrap();
    assert!(everything.is_everything());
    assert_eq!(matched("\u{200b}", "k", "t", &home), Some(Vec::new()));
}

#[test]
fn text_is_compared_lower_cased_and_whitespace_collapsed_code_point_by_code_point() {
    assert_eq!(visible_text("  Caf\u{e9}\t\n TIME  "), "caf\u{e9} time");
    // A final sigma folds as any other sigma (the fold itself: `session_metadata_text`).
    assert_eq!(visible_text("ÀÉÎ ΣΊΣΥΦΟΣ"), "àéî σίσυφοσ");
    let home = SessionHomeScope::LegacyUnscoped;
    assert_eq!(
        matched("CAFÉ", "k", "café notes", &home),
        Some(vec![MatchedField::Title])
    );
    assert_eq!(
        matched("two   words", "k", "words come in two", &home),
        Some(vec![MatchedField::Title])
    );
    // No normalization: a decomposed spelling is a different text, and its
    // combining mark is kept, never stripped into a false match.
    assert_eq!(visible_text("cafe\u{301}"), "cafe\u{301}");
    assert_eq!(matched("caf\u{e9}", "k", "cafe\u{301} notes", &home), None);
    assert_eq!(
        matched("cafe\u{301}", "k", "CAFE\u{301} notes", &home),
        Some(vec![MatchedField::Title])
    );
}

#[test]
fn a_non_utf8_path_is_searchable_by_its_lossy_text_and_never_panics() {
    let dir = PathBuf::from(std::ffi::OsStr::from_bytes(b"/work/caf\xe9/app"));
    let home = folder_home(dir);
    assert_eq!(execution_path(&home).as_deref(), Some("/work/caf\\xE9/app"));
    // R1-H10: two folders that differ in a byte that is no text stay two.
    let other = folder_home(PathBuf::from(std::ffi::OsStr::from_bytes(
        b"/work/caf\xea/app",
    )));
    assert_ne!(execution_path(&home), execution_path(&other));
    let (a, b) = (
        folder_home(PathBuf::from(std::ffi::OsStr::from_bytes(b"/w/caf\xe9"))),
        folder_home(PathBuf::from(std::ffi::OsStr::from_bytes(b"/w/caf\xea"))),
    );
    assert_eq!(repository_label(&a).as_deref(), Some("caf\\xE9"));
    assert_ne!(repository_label(&a), repository_label(&b));
    assert!(matched("caf", "k", "t", &a).is_some() && matched("caf", "k", "t", &b).is_some());
    assert_eq!(
        matched("caf", "k", "t", &home),
        Some(vec![MatchedField::Path])
    );
    assert_eq!(
        matched("app", "k", "t", &home),
        Some(vec![MatchedField::Repository, MatchedField::Path])
    );
}

#[test]
fn the_repository_label_names_the_work_tree_the_bare_repository_or_the_folder() {
    let label = |home: &SessionHomeScope| repository_label(home);
    assert_eq!(
        label(&git_home("/w/quecto/sub", "/w/quecto/.git")).as_deref(),
        Some("quecto")
    );
    // A linked worktree shares the main repository's label.
    assert_eq!(
        label(&git_home("/w/wt-feature", "/w/quecto/.git")).as_deref(),
        Some("quecto")
    );
    assert_eq!(
        label(&git_home("/w/checkout", "/srv/git/tool.git")).as_deref(),
        Some("tool")
    );
    assert_eq!(
        label(&folder_home("/home/u/notes")).as_deref(),
        Some("notes")
    );
    assert_eq!(label(&folder_home("/")), None);
    assert_eq!(label(&git_home("/", "/.git")), None);
}

#[test]
fn an_over_long_query_is_refused_whole_and_the_bound_counts_visible_characters() {
    let fits = "x".repeat(MAX_QUERY_CHARS);
    assert!(MetadataQuery::parse(&fits).is_ok());
    assert!(MetadataQuery::parse(&format!("{fits}\u{200b}\u{200b}")).is_ok());
    let refusal = MetadataQuery::parse(&format!("{fits}y")).unwrap_err();
    assert_eq!(
        refusal,
        QueryRefusal::TooLong {
            chars: MAX_QUERY_CHARS + 1
        }
    );
    assert_eq!(
        refusal.to_string(),
        "query too long: 257 characters (at most 256 are searched)"
    );
    assert!(MetadataQuery::parse(&"é".repeat(100_000)).is_err());
    // R2-H4: counted before the fold — `ß` is one character, though it is
    // searched as `ss`; a whitespace run is the one space it is searched as.
    assert!(MetadataQuery::parse(&"ß".repeat(MAX_QUERY_CHARS)).is_ok());
    assert!(MetadataQuery::parse(&format!("{}   \t  y", "ß".repeat(254))).is_ok());
    assert_eq!(
        MetadataQuery::parse(&"ß".repeat(MAX_QUERY_CHARS + 1)).unwrap_err(),
        QueryRefusal::TooLong { chars: 257 }
    );
    assert_eq!(MetadataQuery::shown("  a\u{200b}  ß\u{7} "), "a ß");
    assert_eq!(MetadataQuery::shown(&"ß".repeat(300)), "ß".repeat(256));
}

#[test]
fn field_names_are_the_wire_spelling_and_rank_is_their_order() {
    let names: Vec<_> = [
        MatchedField::Key,
        MatchedField::Title,
        MatchedField::Repository,
        MatchedField::Path,
    ]
    .iter()
    .map(|field| field.name())
    .collect();
    assert_eq!(names, ["key", "title", "repository", "path"]);
    assert!(
        MatchedField::Key < MatchedField::Title && MatchedField::Repository < MatchedField::Path
    );
}

fn matched_locally(
    query: &str,
    title: &str,
    home: &SessionHomeScope,
    root: &str,
) -> Option<Vec<MatchedField>> {
    let fields = SessionMetadataFields {
        key: "k",
        title,
        home,
        local_root: Some(std::path::Path::new(root)),
    };
    MetadataQuery::parse(query).unwrap().matches(&fields)
}

/// R1-T12: inside one workspace group every row shares the root's path and
/// the repository label, so neither can tell two rows apart.
#[test]
fn a_local_search_matches_the_path_below_the_shared_root_never_the_root_or_the_label() {
    let root = "/home/me/Documents/quecto";
    let sub = git_home(
        "/home/me/Documents/quecto/docs/adr",
        "/home/me/Documents/quecto/.git",
    );
    let top = git_home(
        "/home/me/Documents/quecto",
        "/home/me/Documents/quecto/.git",
    );
    // The shared part — `/home`, `Documents`, the label — matches no row.
    for shared in ["home", "doc", "quecto", "me/Documents"] {
        assert_eq!(matched_locally(shared, "t", &top, root), None, "{shared}");
    }
    assert_eq!(matched_locally("quecto", "t", &sub, root), None);
    // What is below the root still finds its row, as the path field.
    assert_eq!(
        matched_locally("adr", "t", &sub, root),
        Some(vec![MatchedField::Path])
    );
    assert_eq!(
        matched_locally("docs/adr", "t", &sub, root),
        Some(vec![MatchedField::Path])
    );
    // A linked worktree outside the root: what it does not share with it.
    let worktree = git_home(
        "/home/me/Documents/quecto-wt/fix-2010",
        "/home/me/Documents/quecto/.git",
    );
    assert_eq!(
        matched_locally("fix-2010", "t", &worktree, root),
        Some(vec![MatchedField::Path])
    );
    assert_eq!(
        matched_locally("quecto-wt", "t", &worktree, root),
        Some(vec![MatchedField::Path])
    );
    assert_eq!(matched_locally("documents", "t", &worktree, root), None);
    // Title and key are untouched; a global search still sees everything.
    assert_eq!(
        matched_locally("plan", "the plan", &top, root),
        Some(vec![MatchedField::Title])
    );
    assert_eq!(
        matched("quecto", "k", "t", &top),
        Some(vec![MatchedField::Repository, MatchedField::Path])
    );
}

#[test]
fn the_group_root_is_the_work_tree_the_bare_repository_or_the_folder() {
    let root = |home: &SessionHomeScope| match home {
        SessionHomeScope::Scoped(home) => group_root(home).map(PathBuf::from),
        _ => None,
    };
    assert_eq!(
        root(&git_home("/w/a/src", "/w/a/.git")),
        Some("/w/a".into())
    );
    assert_eq!(
        root(&git_home("/w/wt", "/srv/bare.git")),
        Some("/srv/bare.git".into())
    );
    assert_eq!(root(&folder_home("/w/plain")), Some("/w/plain".into()));
}

// ── the title subsequence tier (#2043) ─────────────────────────────────────

fn plain(query: &str, title: &str) -> Option<Vec<MatchedField>> {
    matched(query, "chat-key", title, &SessionHomeScope::LegacyUnscoped)
}

/// A term found nowhere literally may match a TITLE as an in-order
/// subsequence; such a row carries `TitleFuzzy` after its literal fields and
/// ranks by that worst tier. Order matters; a literal term stays literal.
#[test]
fn a_term_found_nowhere_literally_matches_a_title_as_a_subsequence_at_the_lowest_tier() {
    assert_eq!(
        plain("fxbg", "fix bug"),
        Some(vec![MatchedField::TitleFuzzy])
    );
    assert_eq!(plain("fxbg", "bug fix"), None, "in order, or not at all");
    assert_eq!(
        plain("fix fxbg", "fix bug"),
        Some(vec![MatchedField::Title, MatchedField::TitleFuzzy])
    );
    let home = folder_home("/work/otter/p");
    assert_eq!(
        matched("otter fxbg", "chat-key", "fix bug", &home),
        Some(vec![MatchedField::Path, MatchedField::TitleFuzzy])
    );
    assert_eq!(
        rank_of(&[MatchedField::Path, MatchedField::TitleFuzzy]),
        Some(MatchedField::TitleFuzzy),
        "a row that needed a fuzzy term ranks by it"
    );
    assert_eq!(
        rank_of(&[MatchedField::Key, MatchedField::Title]),
        Some(MatchedField::Key)
    );
    assert_eq!(rank_of(&[]), None);
    assert_eq!(MatchedField::TitleFuzzy.name(), "title_fuzzy");
    assert!(
        MatchedField::Path < MatchedField::TitleFuzzy,
        "the lowest tier"
    );
}

/// The tier needs three SHOWN characters (counted as typed, like the length
/// bound: `ß` is one) and is never tried on the key, the repository label or
/// the path.
#[test]
fn subsequence_matching_needs_three_shown_characters_and_touches_titles_only() {
    assert_eq!(plain("fb", "fix bug"), None);
    assert_eq!(
        plain("ßb", "s s b"),
        None,
        "two shown characters, three folded"
    );
    assert_eq!(
        plain("ßb", "ssb"),
        Some(vec![MatchedField::Title]),
        "literal, as ever"
    );
    assert_eq!(
        plain("ßbx", "s s b x"),
        Some(vec![MatchedField::TitleFuzzy])
    );
    // The key holds c, a, b in order; the label o, t, r; the path w, r, k.
    assert_eq!(
        matched(
            "cab",
            "chat-abc",
            "nothing",
            &SessionHomeScope::LegacyUnscoped
        ),
        None
    );
    let home = git_home("/work/otter/p", "/work/otter/.git");
    assert_eq!(matched("otr", "chat-key", "nothing", &home), None);
    assert_eq!(matched("wrk", "chat-key", "nothing", &home), None);
    assert_eq!(
        matched("otr", "chat-key", "other", &home),
        Some(vec![MatchedField::TitleFuzzy]),
        "the title, and only the title"
    );
}

/// Hostile input follows the literal rules: invisible characters are dropped
/// on both sides, code points compare as stored (no normalization), and an
/// over-long query is refused before any tier runs.
#[test]
fn subsequence_matching_is_as_literal_as_the_fold() {
    assert_eq!(
        plain("f\u{200b}xbg", "fix bug"),
        Some(vec![MatchedField::TitleFuzzy])
    );
    assert_eq!(
        plain("cf\u{e9}", "caf\u{e9}"),
        Some(vec![MatchedField::TitleFuzzy])
    );
    assert_eq!(plain("cf\u{e9}", "cafe\u{301}"), None, "no normalization");
    assert_eq!(
        plain("CFE", "cafe"),
        Some(vec![MatchedField::TitleFuzzy]),
        "case folded"
    );
    assert_eq!(
        plain("fxbg fxbg", "fix bug"),
        Some(vec![MatchedField::TitleFuzzy])
    );
    assert!(MetadataQuery::parse(&"x".repeat(300)).is_err());
    assert_eq!(plain("", "fix bug"), Some(vec![]), "everything, no tier");
}

/// #2043 round 1: a typed whole key that also reads as a subsequence of the
/// title stays `[Key]` — never demoted to the fuzzy tier; the same text on a
/// row that is not the key is fuzzy.
#[test]
fn a_typed_whole_key_is_never_demoted_by_the_title_subsequence_tier() {
    let home = SessionHomeScope::LegacyUnscoped;
    assert_eq!(
        matched("cli:work", "cli:work", "cli: work items", &home),
        Some(vec![MatchedField::Key])
    );
    assert_eq!(
        matched("cli:work", "other-key", "cli: work items", &home),
        Some(vec![MatchedField::TitleFuzzy])
    );
}
