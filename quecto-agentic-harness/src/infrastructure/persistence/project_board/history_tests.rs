use super::*;

#[test]
fn source_metadata_32m_is_inclusive_and_checked_overflow_refuses() {
    let mut metadata = META - 512;
    charge(&mut metadata, 512, META).unwrap();
    assert_eq!(metadata, META);
    assert!(matches!(
        charge(&mut metadata, 1, META),
        Err(HistoryError::Bounds(_))
    ));
    let mut overflow = usize::MAX;
    assert!(matches!(
        charge(&mut overflow, 1, usize::MAX),
        Err(HistoryError::Bounds(_))
    ));
    assert_eq!(overflow, usize::MAX);
}

fn tree(id: String, entries: Vec<HistoryEntry>) -> HistoryObject {
    HistoryObject {
        oid: id,
        kind: HistoryKind::Tree,
        bytes: vec![],
        entries,
    }
}
#[test]
fn source_depth128_root_zero_is_inclusive_and_reused_deeper_tree_refuses() {
    let mut objects = BTreeMap::new();
    let mut previous = "0".repeat(40);
    objects.insert(previous.clone(), tree(previous.clone(), vec![]));
    for n in 1..=128 {
        let id = format!("{n:040x}");
        objects.insert(
            id.clone(),
            tree(
                id.clone(),
                vec![HistoryEntry {
                    mode: "040000".into(),
                    name: b"next".to_vec(),
                    oid: previous,
                }],
            ),
        );
        previous = id;
    }
    let mut heights = BTreeMap::new();
    assert_eq!(
        height(&previous, &objects, &mut heights, &mut BTreeSet::new(), 0).unwrap(),
        128
    );
    assert!(matches!(
        height(&previous, &objects, &mut heights, &mut BTreeSet::new(), 1),
        Err(HistoryError::Bounds(_))
    ));
}
#[test]
fn source_hashes_full_actual_kind_framing_both_formats() {
    for (width, id) in [
        (40, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
        (
            64,
            "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813",
        ),
    ] {
        let mut object = HistoryObject {
            oid: id.into(),
            kind: HistoryKind::Blob,
            bytes: vec![],
            entries: vec![],
        };
        verify_raw(&object, width).unwrap();
        object.kind = HistoryKind::Tree;
        assert!(matches!(
            verify_raw(&object, width),
            Err(HistoryError::Malformed(_))
        ));
        object.kind = HistoryKind::Blob;
        object.bytes.push(0);
        assert!(matches!(
            verify_raw(&object, width),
            Err(HistoryError::Malformed(_))
        ));
    }
}
#[test]
fn source_projection_preallocation_checks_frames_and_name_bounds() {
    let raw = |n| {
        let mut b = b"100644 ".to_vec();
        b.extend(vec![b'x'; n]);
        b.push(0);
        b.extend([0; 20]);
        b
    };
    assert_eq!(
        projection_budget(&raw(4096), 40).unwrap(),
        (1, 128 + 6 + 40 + 4 * 4096)
    );
    assert!(matches!(
        projection_budget(&raw(4097), 40),
        Err(HistoryError::Bounds(_))
    ));
    let mut truncated = raw(1);
    truncated.pop();
    assert!(matches!(
        projection_budget(&truncated, 40),
        Err(HistoryError::Malformed(_))
    ));
}

#[test]
fn source_required_identity_headers_refuse_empty_name_and_email() {
    for identity in [" <h@example.com>", "History <>"] {
        let bytes = format!(
            "tree {}\nauthor {identity} 1 +0000\ncommitter History <h@example.com> 1 +0000\n\nbody",
            "0".repeat(40)
        );
        assert!(matches!(
            commit_links(bytes.as_bytes(), 40),
            Err(HistoryError::Malformed(_))
        ));
    }
}

/// A real git in `dir`, isolated from the user's config, whatever its exit.
fn scratch(dir: &std::path::Path, args: &[&str], input: &[u8]) -> std::process::Output {
    use std::io::Write;
    let mut child = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", "/nonexistent"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

/// [`scratch`]'s trimmed output, which must succeed.
fn scratch_ok(dir: &std::path::Path, args: &[&str], input: &[u8]) -> String {
    let output = scratch(dir, args, input);
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim_end().into()
}

/// A bare destination of `format`, its sink, and the directory holding it.
fn destination(format: &str) -> (tempfile::TempDir, PathBuf, GitBoardHistorySink) {
    let dir = tempfile::tempdir().unwrap();
    let flag = format!("--object-format={format}");
    scratch_ok(dir.path(), &["init", "-q", "--bare", &flag, "d.git"], b"");
    let repo = dir.path().join("d.git");
    let sink = GitBoardHistorySink::new(repo.clone(), "refs/heads/quecto/rollback".into());
    (dir, repo, sink)
}

fn kinds() -> [(HistoryKind, &'static str, Vec<u8>); 3] {
    let mut tree = b"100644 a.md\0".to_vec();
    tree.extend([7u8; 20]);
    [
        (HistoryKind::Blob, "blob", b"blob body\n".to_vec()),
        // Literal objects git would refuse to fsck still hash and store as given.
        (HistoryKind::Tree, "tree", tree),
        (
            HistoryKind::Commit,
            "commit",
            b"tree 0\n\nnot a well-formed commit\n".to_vec(),
        ),
    ]
}

#[test]
fn sink_ids_match_gits_own_literal_hash_for_every_kind_in_both_formats() {
    for format in ["sha1", "sha256"] {
        let (_dir, repo, sink) = destination(format);
        for (kind, name, bytes) in kinds() {
            let ours = sink.raw(kind, &bytes, None, true).unwrap();
            let args = ["hash-object", "--literally", "-t", name, "--stdin"];
            assert_eq!(ours, scratch_ok(&repo, &args, &bytes), "{format} {name}");
        }
    }
}

#[test]
fn sink_dry_run_hashes_without_running_git_once_the_format_is_known() {
    let (_dir, repo, sink) = destination("sha256");
    assert_eq!(sink.algorithm().unwrap(), Some(HistoryAlgorithm::Sha256));
    let (kind, name, bytes) = kinds().into_iter().next().unwrap();
    let args = ["hash-object", "--literally", "-t", name, "--stdin"];
    let expected = scratch_ok(&repo, &args, &bytes);
    std::fs::remove_dir_all(&repo).unwrap();
    assert_eq!(sink.raw(kind, &bytes, None, true).unwrap(), expected);
    assert!(matches!(
        sink.raw(kind, &bytes, Some(&"0".repeat(64)), true),
        Err(HistoryError::Malformed(_))
    ));
}

#[test]
fn sink_stores_every_written_object_exactly_before_it_publishes() {
    for format in ["sha1", "sha256"] {
        let (_dir, repo, sink) = destination(format);
        let mut ids = vec![];
        for (kind, name, bytes) in kinds() {
            let id = sink.raw(kind, &bytes, None, false).unwrap();
            // The same object written twice is stored once, under one id.
            assert_eq!(sink.raw(kind, &bytes, Some(&id), false).unwrap(), id);
            ids.push((id, name, bytes));
        }
        let entry = HistoryEntry {
            mode: "100644".into(),
            name: b"a.md".to_vec(),
            oid: ids[0].0.clone(),
        };
        let tree = sink.tree(&[entry], false).unwrap();
        let writer = crate::domain::project_board::entities::claim::Identity {
            name: "History".into(),
            email: "history@example.com".into(),
        };
        let revision = HistoryCommit {
            tree: &tree,
            parent: None,
            writer: &writer,
            message: "board: export",
            at: 1700000000,
        };
        let head = sink.commit(revision, false).unwrap();
        sink.publish(None, &head).unwrap();
        for (id, name, bytes) in &ids {
            assert_eq!(scratch_ok(&repo, &["cat-file", "-t", id], b""), *name);
            let body = scratch(&repo, &["cat-file", name, id], b"");
            assert_eq!(&body.stdout, bytes, "{format} {name}");
        }
        let target = ["rev-parse", "refs/heads/quecto/rollback"];
        assert_eq!(scratch_ok(&repo, &target, b""), head);
        let root = format!("{head}^{{tree}}");
        assert_eq!(scratch_ok(&repo, &["rev-parse", &root], b""), tree);
    }
}

#[test]
fn sink_stores_a_full_batch_without_waiting_for_publication() {
    let (_dir, repo, sink) = destination("sha1");
    let ids: Vec<_> = (0..=WRITE_BATCH)
        .map(|n| {
            let bytes = n.to_string();
            sink.raw(HistoryKind::Blob, bytes.as_bytes(), None, false)
                .unwrap()
        })
        .collect();
    let stored = |id: &str| {
        scratch(&repo, &["cat-file", "-e", id], b"")
            .status
            .success()
    };
    assert!(ids[..WRITE_BATCH].iter().all(|id| stored(id)));
    assert!(!stored(&ids[WRITE_BATCH]), "the next batch waits");
}

#[test]
fn sink_stores_a_batch_before_it_would_pass_its_byte_bound() {
    let (_dir, repo, sink) = destination("sha1");
    let half = vec![b'a'; WRITE_BATCH_BYTES / 2];
    let mut more = half.clone();
    more.push(b'b');
    let first = sink.raw(HistoryKind::Blob, &half, None, false).unwrap();
    let second = sink.raw(HistoryKind::Blob, &more, None, false).unwrap();
    let stored = |id: &str| {
        scratch(&repo, &["cat-file", "-e", id], b"")
            .status
            .success()
    };
    assert!(
        stored(&first),
        "a batch past its byte bound is stored first"
    );
    assert!(!stored(&second));
}

#[test]
fn sink_lists_only_plain_absolute_paths_to_git() {
    assert!(listable(std::path::Path::new("/tmp/quecto-export-1/ab")).is_ok());
    for path in ["relative/ab", "/tmp/a\nb", "/tmp/a\rb", "/tmp/a\u{7f}b"] {
        let path = std::path::Path::new(path);
        assert!(
            matches!(listable(path), Err(HistoryError::Unavailable(_))),
            "{path:?}"
        );
    }
}
