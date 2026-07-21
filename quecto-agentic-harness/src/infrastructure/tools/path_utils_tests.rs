use super::*;
use tempfile::TempDir;

fn tmp() -> TempDir {
    TempDir::new().unwrap()
}

// --- normalize_at_prefix ---

#[test]
fn strips_at_prefix() {
    assert_eq!(normalize_at_prefix("@src/main.rs"), "src/main.rs");
}

#[test]
fn no_at_prefix_unchanged() {
    assert_eq!(normalize_at_prefix("src/main.rs"), "src/main.rs");
}

#[test]
fn double_at_strips_one() {
    assert_eq!(normalize_at_prefix("@@foo"), "@foo");
}

// --- expand_tilde ---

#[test]
fn tilde_alone_expands_to_home() {
    if let Some(home) = home_dir() {
        let result = expand_tilde("~");
        assert_eq!(result, home);
    }
}

#[test]
fn tilde_slash_expands() {
    if let Some(home) = home_dir() {
        let result = expand_tilde("~/foo/bar");
        assert_eq!(result, home.join("foo/bar"));
    }
}

#[test]
fn no_tilde_unchanged() {
    let result = expand_tilde("foo/bar");
    assert_eq!(result, PathBuf::from("foo/bar"));
}

#[test]
fn absolute_path_unchanged_by_tilde_expand() {
    let result = expand_tilde("/usr/bin/ls");
    assert_eq!(result, PathBuf::from("/usr/bin/ls"));
}

// --- normalize_unicode_spaces ---

#[test]
fn nbsp_normalised() {
    assert_eq!(
        normalize_unicode_spaces("my\u{00A0}file.txt"),
        "my file.txt"
    );
}

#[test]
fn narrow_nbsp_normalised() {
    assert_eq!(
        normalize_unicode_spaces("my\u{202F}file.txt"),
        "my file.txt"
    );
}

#[test]
fn regular_space_unchanged_borrowed() {
    let s = "my file.txt";
    let result = normalize_unicode_spaces(s);
    assert!(matches!(result, Cow::Borrowed(_)));
    assert_eq!(result, s);
}

#[test]
fn ascii_fast_path_borrowed() {
    let s = "hello_world.rs";
    let result = normalize_unicode_spaces(s);
    assert!(matches!(result, Cow::Borrowed(_)));
}

#[test]
fn special_space_produces_owned() {
    let s = "my\u{3000}file.txt";
    let result = normalize_unicode_spaces(s);
    assert!(matches!(result, Cow::Owned(_)));
    assert_eq!(result, "my file.txt");
}

// --- resolve_to_cwd ---

#[test]
fn relative_path_resolved_against_cwd() {
    let td = tmp();
    let result = resolve_to_cwd("sub/file.txt", td.path());
    assert_eq!(result, td.path().join("sub/file.txt"));
}

#[test]
fn absolute_path_returned_as_is() {
    let td = tmp();
    let result = resolve_to_cwd("/etc/hosts", td.path());
    assert_eq!(result, PathBuf::from("/etc/hosts"));
}

#[test]
fn tilde_resolved() {
    if let Some(home) = home_dir() {
        let td = tmp();
        let result = resolve_to_cwd("~/foo.txt", td.path());
        assert_eq!(result, home.join("foo.txt"));
    }
}

#[test]
fn at_prefix_stripped_then_resolved() {
    let td = tmp();
    let result = resolve_to_cwd("@src/main.rs", td.path());
    assert_eq!(result, td.path().join("src/main.rs"));
}

#[test]
fn nbsp_normalised_in_path() {
    let td = tmp();
    let result = resolve_to_cwd("my\u{00A0}file.txt", td.path());
    assert_eq!(result, td.path().join("my file.txt"));
}

#[test]
fn dot_resolves_under_cwd() {
    let td = tmp();
    let result = resolve_to_cwd(".", td.path());
    assert_eq!(result, td.path().join("."));
}

// --- resolve_read_path ---

#[test]
fn existing_file_returned_directly() {
    let td = tmp();
    std::fs::write(td.path().join("readme.md"), "hello").unwrap();
    let result = resolve_read_path("readme.md", td.path());
    assert_eq!(result, td.path().join("readme.md"));
    assert!(result.exists());
}

#[test]
fn non_existent_file_returns_primary() {
    let td = tmp();
    let result = resolve_read_path("missing.txt", td.path());
    assert_eq!(result, td.path().join("missing.txt"));
}

#[test]
fn curly_right_quote_variant_found() {
    let td = tmp();
    let stored = td.path().join("Capture d'écran.png");
    std::fs::write(&stored, b"").unwrap();
    // Query with curly right quote (U+2019)
    let result = resolve_read_path("Capture d\u{2019}\u{E9}cran.png", td.path());
    assert_eq!(result, stored);
}

#[test]
fn home_dir_cached() {
    // Call twice — should return same pointer (OnceLock)
    let a = home_dir();
    let b = home_dir();
    assert_eq!(a, b);
}
