use super::*;
use crate::application::project_board::ports::replication::FollowTokens;
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

/// The file the tokens are in.
fn tokens_path(tokens: &FileBoardTokens) -> PathBuf {
    tokens.state.join(TOKENS_FILE)
}

fn sha256_hex(text: &str) -> String {
    sha256(text)
}

/// B5 red: the file holds each token's SHA-256, never the token; it is
/// mode 0600 in a directory of mode 0700.
#[test]
fn tokens_are_stored_hashed_in_a_private_file() {
    let (_dir, tokens) = opened();
    let issued = tokens.add(&name("laptop"), &at()).unwrap();
    assert_eq!(issued.secret.len(), 43, "32 bytes, base64url: {issued:?}");
    let raw = std::fs::read_to_string(tokens_path(&tokens)).unwrap();
    assert!(!raw.contains(&issued.secret), "{raw}");
    assert!(raw.contains(&sha256_hex(&issued.secret)), "{raw}");
    assert!(raw.contains("laptop") && raw.contains("2026-10-24T12:00:00Z"));
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&tokens_path(&tokens)), 0o600);
    assert_eq!(mode(tokens_path(&tokens).parent().unwrap()), 0o700);
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

/// Review round 1: two servers on one state directory lost tokens (80
/// parallel adds kept 41). The state directory's lock is held for the
/// adapter's life, so a second one is refused at start, until the first
/// is gone.
#[test]
fn a_second_server_on_one_state_directory_is_refused() {
    let (dir, tokens) = opened();
    let state = dir.path().join("state").join("board");
    let error = FileBoardTokens::open(&state).unwrap_err();
    assert!(error.contains("another board server holds it"), "{error}");
    assert!(error.contains("--state"), "{error}");
    drop(tokens);
    assert!(FileBoardTokens::open(&state).is_ok());
}

/// Review round 2: deleting a lock file let a second server in. The lock
/// is on the directory itself, so there is no file to delete; and a
/// directory moved away and replaced (which a second server could then
/// lock) stops the first one's writes.
#[test]
fn the_lock_is_the_directory_and_a_replaced_one_stops_writes() {
    let (dir, tokens) = opened();
    let state = dir.path().join("state").join("board");
    tokens.add(&name("laptop"), &at()).unwrap();
    let entries: Vec<_> = std::fs::read_dir(&state)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, [TOKENS_FILE], "no lock file to remove");
    std::fs::rename(&state, dir.path().join("state").join("moved")).unwrap();
    let second = FileBoardTokens::open(&state).unwrap();
    let refused = tokens.add(&name("phone"), &at()).unwrap_err();
    assert!(
        matches!(&refused, TokenError::Unavailable(why) if why.contains("moved or replaced")),
        "{refused:?}"
    );
    assert!(matches!(
        tokens.revoke(&name("laptop")),
        Err(TokenError::Unavailable(_))
    ));
    second.add(&name("phone"), &at()).unwrap();
}

/// Within one server, parallel adds are serialised: every one is kept.
#[test]
fn parallel_adds_through_one_server_are_all_kept() {
    let (_dir, tokens) = opened();
    let tokens = std::sync::Arc::new(tokens);
    let adders: Vec<_> = (0..80)
        .map(|n| {
            let tokens = std::sync::Arc::clone(&tokens);
            std::thread::spawn(move || tokens.add(&name(&format!("d{n}")), &at()).unwrap())
        })
        .collect();
    let issued: Vec<IssuedToken> = adders.into_iter().map(|a| a.join().unwrap()).collect();
    assert_eq!(tokens.list().unwrap().len(), 80);
    for token in &issued {
        assert_eq!(
            tokens.verify(&token.secret),
            Ok(Some(token.record.name.clone()))
        );
    }
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
    let path = tokens_path(&tokens).to_path_buf();
    let state = dir.path().join("state").join("board");
    drop(tokens);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
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
        let error = FileBoardTokens::open(&state).unwrap_err();
        assert!(error.contains("the tokens file"), "{bad}: {error}");
    }
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(dir.path().join("elsewhere"), &path).unwrap();
    let error = FileBoardTokens::open(&state).unwrap_err();
    assert!(
        error.contains("the tokens file"),
        "a symlink is not followed: {error}"
    );
}

/// Review rounds 3 and 4: the server reads the file once and verifies
/// from memory. A hand edit behind it is never overwritten: its next
/// change is refused (restart to pick the edit up), memory unchanged.
#[test]
fn a_hand_edit_behind_the_server_is_refused_never_undone() {
    let (_dir, tokens) = opened();
    let laptop = tokens.add(&name("laptop"), &at()).unwrap();
    tokens.add(&name("phone"), &at()).unwrap();
    let edited = format!("[{}]", "");
    std::fs::write(tokens_path(&tokens), &edited).unwrap();
    assert_eq!(tokens.verify(&laptop.secret), Ok(Some(name("laptop"))));
    for refused in [
        tokens.add(&name("tablet"), &at()).map(|_| ()),
        tokens.revoke(&name("laptop")),
    ] {
        let Err(TokenError::Unavailable(why)) = refused else {
            panic!("{refused:?}")
        };
        assert!(why.contains("restart it to pick up hand edits"), "{why}");
    }
    assert_eq!(
        std::fs::read_to_string(tokens_path(&tokens)).unwrap(),
        edited
    );
    assert_eq!(tokens.list().unwrap().len(), 2);
    assert_eq!(tokens.verify(&laptop.secret), Ok(Some(name("laptop"))));
}

/// Review round 4: a write that fails changes nothing: with the state
/// directory read-only, add and revoke fail, the list is unchanged and the
/// token still verifies (keeping a change before it lands would fail this).
#[test]
fn a_failed_write_leaves_the_tokens_unchanged() {
    let (dir, tokens) = opened();
    let laptop = tokens.add(&name("laptop"), &at()).unwrap();
    let state = dir.path().join("state").join("board");
    let mode = |mode| std::fs::set_permissions(&state, std::fs::Permissions::from_mode(mode));
    mode(0o500).unwrap();
    let added = tokens.add(&name("phone"), &at());
    let revoked = tokens.revoke(&name("laptop"));
    mode(0o700).unwrap();
    assert!(
        matches!(added, Err(TokenError::Unavailable(_))),
        "{added:?}"
    );
    assert!(
        matches!(revoked, Err(TokenError::Unavailable(_))),
        "{revoked:?}"
    );
    let names: Vec<_> = tokens.list().unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(names, [name("laptop")]);
    assert_eq!(tokens.verify(&laptop.secret), Ok(Some(name("laptop"))));
    tokens.add(&name("phone"), &at()).unwrap();
}

/// Review round 3: a `--state` ending in `/` would follow a symlink, so it
/// is refused.
#[test]
fn a_state_directory_ending_in_a_slash_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let slashed = PathBuf::from(format!("{}/", link.display()));
    let error = FileBoardTokens::open(&slashed).unwrap_err();
    assert!(error.contains("must not end in '/'"), "{error}");
    assert!(
        FileBoardTokens::open(&link)
            .unwrap_err()
            .contains("must be a directory")
    );
    let real_slashed = PathBuf::from(format!("{}/", real.display()));
    assert!(FileBoardTokens::open(&real_slashed).is_err());
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
            .contains("must be a directory")
    );
    assert!(
        FileBoardTokens::open(Path::new("state"))
            .unwrap_err()
            .contains("not absolute")
    );
}

#[test]
fn digests_spell_as_lowercase_hex() {
    assert_eq!(
        sha256("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let entry = |sha256: &str| StoredToken {
        name: "laptop".into(),
        sha256: sha256.into(),
        created: "2026-10-24T12:00:00Z".into(),
        replica: false,
    };
    assert!(held(entry(&sha256("x"))).is_some());
    assert!(held(entry(&"A".repeat(64))).is_none());
    assert!(held(entry("00")).is_none());
}

/// B11 review 4: a crash while a follower swaps in its primary's next set
/// (after the digests are noted, before or after tokens.json is replaced)
/// leaves a set a restart knows as its primary's, never as its own.
#[test]
fn a_crash_while_a_set_is_swapped_in_leaves_a_primarys_set() {
    let (dir, primary) = opened();
    let first = primary.add(&name("phone"), &at()).unwrap();
    let state = dir.path().join("follower");
    let follower = FileBoardTokens::open(&state).unwrap();
    follower.replace(&primary.export().unwrap()).unwrap();
    let older = follower.export().unwrap();
    primary.add(&name("laptop"), &at()).unwrap();
    let mut newer = primary.export().unwrap();
    newer.sort_by(|a, b| a.name.cmp(&b.name));
    // The swap's first write: both sets noted.
    let noted = vec![set_digest(&newer), set_digest(&older)];
    follower.keep_copied(&noted).unwrap();
    for swapped in [false, true] {
        if swapped {
            // The swap's second write: tokens.json holds the newer set.
            let rows: Vec<Held> = newer
                .iter()
                .map(|row| Held {
                    record: TokenRecord {
                        name: row.name.clone(),
                        created: row.created.clone(),
                        kind: row.kind,
                    },
                    digest: row.digest.clone(),
                })
                .collect();
            follower
                .changing(|held| {
                    *held = rows;
                    Ok(())
                })
                .unwrap();
        }
        let held = follower.followed().unwrap();
        assert!(
            held.copied,
            "a primary's set (swapped: {swapped}): {held:?}"
        );
    }
    drop(follower);
    let reopened = FileBoardTokens::open(&state).unwrap();
    assert!(reopened.followed().unwrap().copied);
    assert_eq!(reopened.verify(&first.secret), Ok(Some(name("phone"))));
}
