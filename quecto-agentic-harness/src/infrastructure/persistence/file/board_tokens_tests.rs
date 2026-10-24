use super::*;
use std::os::unix::fs::PermissionsExt;

fn name(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at() -> Timestamp {
    Timestamp::parse("2026-10-24T12:00:00Z").unwrap()
}

/// A state directory inside a temp dir, and the tokens opened on it.
fn opened() -> (tempfile::TempDir, FileBoardTokens) {
    let dir = tempfile::tempdir().unwrap();
    let tokens = FileBoardTokens::open(&dir.path().join("state").join("board")).unwrap();
    (dir, tokens)
}

fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()).into())
}

/// B5 red: the file holds each token's SHA-256, never the token; it is
/// mode 0600 in a directory of mode 0700.
#[test]
fn tokens_are_stored_hashed_in_a_private_file() {
    let (_dir, tokens) = opened();
    let issued = tokens.add(&name("laptop"), &at()).unwrap();
    assert_eq!(issued.secret.len(), 43, "32 bytes, base64url: {issued:?}");
    let raw = std::fs::read_to_string(tokens.path()).unwrap();
    assert!(!raw.contains(&issued.secret), "{raw}");
    assert!(raw.contains(&sha256_hex(&issued.secret)), "{raw}");
    assert!(raw.contains("laptop") && raw.contains("2026-10-24T12:00:00Z"));
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(tokens.path()), 0o600);
    assert_eq!(mode(tokens.path().parent().unwrap()), 0o700);
}

#[test]
fn a_held_token_verifies_and_a_revoked_one_no_longer_does() {
    let (_dir, tokens) = opened();
    let laptop = tokens.add(&name("laptop"), &at()).unwrap();
    let phone = tokens.add(&name("phone"), &at()).unwrap();
    assert_ne!(laptop.secret, phone.secret);
    assert_eq!(tokens.verify(&laptop.secret), Ok(Some(name("laptop"))));
    assert_eq!(tokens.verify(&phone.secret), Ok(Some(name("phone"))));
    assert_eq!(tokens.verify("not-a-token"), Ok(None));
    assert_eq!(tokens.verify(""), Ok(None));
    tokens.revoke(&name("laptop")).unwrap();
    assert_eq!(tokens.verify(&laptop.secret), Ok(None));
    assert_eq!(tokens.verify(&phone.secret), Ok(Some(name("phone"))));
    assert_eq!(
        tokens.revoke(&name("laptop")),
        Err(TokenError::NoSuchToken(name("laptop")))
    );
}

#[test]
fn a_taken_name_is_refused_and_the_list_is_names_and_dates() {
    let (_dir, tokens) = opened();
    tokens.add(&name("phone"), &at()).unwrap();
    tokens.add(&name("laptop"), &at()).unwrap();
    assert_eq!(
        tokens.add(&name("laptop"), &at()),
        Err(TokenError::Exists(name("laptop")))
    );
    let listed = tokens.list().unwrap();
    let names: Vec<&str> = listed.iter().map(|token| token.name.as_str()).collect();
    assert_eq!(names, ["laptop", "phone"]);
}

#[test]
fn a_full_store_refuses_another_token() {
    let (_dir, tokens) = opened();
    for n in 0..MAX_TOKENS {
        tokens.add(&name(&format!("d{n}")), &at()).unwrap();
    }
    assert_eq!(tokens.add(&name("one-more"), &at()), Err(TokenError::Full));
}

/// The tokens survive the server: a fresh open reads what was written.
#[test]
fn tokens_outlive_the_adapter() {
    let (dir, tokens) = opened();
    let issued = tokens.add(&name("laptop"), &at()).unwrap();
    drop(tokens);
    let reopened = FileBoardTokens::open(&dir.path().join("state").join("board")).unwrap();
    assert_eq!(reopened.verify(&issued.secret), Ok(Some(name("laptop"))));
}

/// A file that is shared, foreign-shaped or not as written is refused,
/// never read as empty (so no token is silently lost or let in).
#[test]
fn a_file_not_as_written_is_refused() {
    let (dir, tokens) = opened();
    tokens.add(&name("laptop"), &at()).unwrap();
    let path = tokens.path().to_path_buf();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        matches!(tokens.verify("x"), Err(TokenError::Unavailable(why)) if why.contains("0600"))
    );
    let state = dir.path().join("state").join("board");
    assert!(FileBoardTokens::open(&state).unwrap_err().contains("0600"));
    for bad in [
        "not json",
        r#"{"tokens":[{"name":"Laptop","sha256":"00","created":"2026-10-24T12:00:00Z"}]}"#,
        r#"{"tokens":[],"extra":1}"#,
    ] {
        std::fs::remove_file(&path).unwrap();
        crate::infrastructure::persistence::file::atomic_write::atomic_write(
            &path,
            bad.as_bytes(),
            Some(0o600),
        )
        .unwrap();
        assert!(
            matches!(tokens.list(), Err(TokenError::Unavailable(_))),
            "{bad}"
        );
        assert!(tokens.add(&name("phone"), &at()).is_err(), "{bad}");
    }
}

#[test]
fn a_shared_or_relative_state_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(FileBoardTokens::open(&shared).unwrap_err().contains("0700"));
    let file = dir.path().join("file");
    std::fs::write(&file, "x").unwrap();
    assert!(
        FileBoardTokens::open(&file)
            .unwrap_err()
            .contains("not a directory")
    );
    assert!(
        FileBoardTokens::open(Path::new("state"))
            .unwrap_err()
            .contains("not absolute")
    );
}

#[test]
fn digests_spell_as_lowercase_hex() {
    let a = [0xa7u8; 32];
    assert_eq!(hex(&a), "a7".repeat(32));
    assert_eq!(unhex(&hex(&a)), Some(a));
    assert_eq!(unhex(&"A".repeat(64)), None);
    assert_eq!(unhex("00"), None);
}
