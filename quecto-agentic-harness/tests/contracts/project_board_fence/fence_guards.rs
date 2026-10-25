use super::*;
use std::process::Command;

fn marked(r: &Remote, marker: &[u8], moved_bytes: Option<&[u8]>) -> BoardHead {
    let marker = String::from_utf8(git(&r.bare, &["hash-object", "-w", "--stdin"], marker))
        .unwrap()
        .trim()
        .to_owned();
    let mut listing = format!("100644 blob {marker}\tboard\n");
    if let Some(bytes) = moved_bytes {
        let blob = String::from_utf8(git(&r.bare, &["hash-object", "-w", "--stdin"], bytes))
            .unwrap()
            .trim()
            .to_owned();
        listing = format!("100644 blob {blob}\tMOVED\n{listing}");
    }
    let format = String::from_utf8(git(&r.bare, &["mktree"], listing.as_bytes()))
        .unwrap()
        .trim()
        .to_owned();
    r.install_root(vec![("format", format!("040000 tree {format}"))])
}
#[test]
fn r7_publication_refuses_malformed_moved_without_repair() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let head = marked(
        &r,
        b"quecto-board 4\n",
        Some(b"board_id ../escape\nurl https://example.test\n"),
    );
    assert!(matches!(
        r.store(1).read(),
        Err(BoardStoreError::MovedMalformed(_))
    ));
    assert!(matches!(
        r.store(0).read(),
        Err(BoardStoreError::MovedMalformed(_))
    ));
    let unrelated = r.unrelated();
    let objects = r.objects();
    let result = r.publish(&head, &moved());
    assert_eq!(r.head(), head);
    assert_eq!(r.unrelated(), unrelated);
    assert_eq!(r.objects(), objects);
    assert!(
        matches!(
            result,
            Err(FenceError::Store(BoardStoreError::MovedMalformed(_)))
        ),
        "operational publication preserves existing malformed refusal: {result:?}"
    );
}
#[test]
fn r7_publication_refuses_unsupported_marker_without_repair() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let head = marked(&r, b"quecto-board 5\n", None);
    assert!(matches!(
        r.store(1).read(),
        Err(BoardStoreError::Unavailable(_))
    ));
    let unrelated = r.unrelated();
    let result = r.publish(&head, &moved());
    assert_eq!(r.head(), head);
    assert_eq!(r.unrelated(), unrelated);
    assert!(
        matches!(
            result,
            Err(FenceError::Store(BoardStoreError::Unavailable(_)))
        ),
        "existing unsupported-format refusal not missing publication capability: {result:?}"
    );
}
#[cfg(unix)]
#[test]
fn r7_exact_repeat_never_pushes_or_looks_up_fresh_commit_identity() {
    use std::os::unix::fs::PermissionsExt;
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let head = r.seed_fence();
    let objects = r.objects();
    let dir = r._dir.path().join("identity-push-trap");
    std::fs::create_dir(&dir).unwrap();
    let original_path = std::env::var_os("PATH").unwrap();
    let real = std::env::split_paths(&original_path)
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .unwrap();
    let real = std::fs::canonicalize(real).unwrap();
    let quote = |p: &std::path::Path| format!("'{}'", p.to_str().unwrap().replace('\'', "'\"'\"'"));
    let log = dir.join("forbidden-calls");
    std::fs::write(&log, b"").unwrap();
    let script = format!(
        "#!/bin/sh\nulimit -c 0\ncase \" $* \" in\n *\" push \"*) printf 'push\\n' >> {}; exit 78;;\n *\" var GIT_AUTHOR_IDENT \"*|*\" var GIT_COMMITTER_IDENT \"*|*\" user.name \"*|*\" user.email \"*) printf 'identity\\n' >> {}; exit 78;;\nesac\nexec timeout --signal=KILL 10 {} \"$@\"\n",
        quote(&log),
        quote(&log),
        quote(&real)
    );
    let wrapper = dir.join("git");
    std::fs::write(&wrapper, script).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path =
        std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&original_path)))
            .unwrap();
    // Independently prove the identity trap is live; then clear fixture log.
    let probe = Command::new("timeout")
        .args(["--signal=KILL", "10"])
        .arg(&wrapper)
        .args(["var", "GIT_AUTHOR_IDENT"])
        .current_dir(&r.clones[0])
        .env_clear()
        .env("PATH", &path)
        .env("HOME", r._dir.path())
        .output()
        .unwrap();
    assert_eq!(probe.status.code(), Some(78));
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "identity\n");
    std::fs::write(&log, b"").unwrap();
    let vars: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), path),
        ("HOME".into(), r._dir.path().into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    let store = GitBoardStore::new(
        Git::reader_unasked(&r.clones[0], GitEnvironment::allowlisted(vars)).unwrap(),
        "origin",
    )
    .unwrap();
    let result = BoardFence::publish_fence(&store, &head, &moved());
    assert_eq!(result, Ok(FenceReport::Already { head: head.clone() }));
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "",
        "Already must not even attempt redundant same-OID push or fresh identity lookup"
    );
    assert_eq!(r.head(), head);
    assert_eq!(r.objects(), objects);
}
