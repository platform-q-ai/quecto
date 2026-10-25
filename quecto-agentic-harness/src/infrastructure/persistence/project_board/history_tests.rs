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
            assert_eq!(sink.raw(kind, &bytes, Some(&id), false).unwrap(), id);
            ids.push((id, name, bytes));
        }
        // The same object written twice is staged, so stored, once.
        let order: Vec<_> = ids.iter().map(|(id, ..)| id.clone()).collect();
        assert_eq!(sink.staged.lock().unwrap().order(), order);
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

/// Whether `repo` holds object `id`.
fn stored(repo: &std::path::Path, id: &str) -> bool {
    scratch(repo, &["cat-file", "-e", id], b"").status.success()
}

/// Makes git store something else for staged `id`: its stored ID then
/// differs from ours, as a corrupted or swapped object would.
fn corrupt(sink: &GitBoardHistorySink, id: &str) {
    let file = sink.staged.lock().unwrap().file(id).unwrap();
    std::fs::write(file, b"not the hashed bytes").unwrap();
}

/// A commit stored in `repo` by git itself, naming an empty tree.
fn outside_commit(repo: &std::path::Path, message: &str) -> String {
    let tree = scratch_ok(repo, &["hash-object", "-w", "-t", "tree", "--stdin"], b"");
    let body = format!(
        "tree {tree}\nauthor A <a@example.com> 1 +0000\ncommitter A <a@example.com> 1 +0000\n\n{message}\n"
    );
    let args = ["hash-object", "-w", "-t", "commit", "--stdin"];
    scratch_ok(repo, &args, body.as_bytes())
}

const TARGET: &str = "refs/heads/quecto/rollback";

#[test]
fn sink_stores_blobs_then_trees_then_commits_whatever_order_they_came_in() {
    let (_dir, repo, sink) = destination("sha1");
    let mut staged = vec![];
    for (kind, _, bytes) in kinds().into_iter().rev() {
        staged.push(sink.raw(kind, &bytes, None, false).unwrap());
    }
    let [commit, tree, blob] = <[String; 3]>::try_from(staged).unwrap();
    assert_eq!(
        sink.staged.lock().unwrap().order(),
        [blob.clone(), tree.clone(), commit.clone()]
    );
    // A tree that fails to store stops the batch before any commit.
    corrupt(&sink, &tree);
    assert!(sink.publish(None, &commit).is_err());
    assert!(stored(&repo, &blob));
    assert!(!stored(&repo, &tree));
    assert!(
        !stored(&repo, &commit),
        "no commit is stored without its tree"
    );
}

#[test]
fn a_failed_store_before_publication_leaves_the_target_ref_alone() {
    let (_dir, repo, sink) = destination("sha1");
    let old = outside_commit(&repo, "published before");
    scratch_ok(&repo, &["update-ref", TARGET, &old], b"");
    let (kind, _, bytes) = kinds().into_iter().next().unwrap();
    let blob = sink.raw(kind, &bytes, None, false).unwrap();
    corrupt(&sink, &blob);
    let new = outside_commit(&repo, "would be published");
    assert!(matches!(
        sink.publish(Some(&old), &new),
        Err(HistoryError::Malformed(_))
    ));
    assert_eq!(scratch_ok(&repo, &["rev-parse", TARGET], b""), old);
    // Nothing is left staged, yet the sink still never publishes.
    assert!(matches!(
        sink.publish(Some(&old), &new),
        Err(HistoryError::Unavailable(why)) if why.contains("earlier object store")
    ));
    assert_eq!(scratch_ok(&repo, &["rev-parse", TARGET], b""), old);
    let fresh = GitBoardHistorySink::new(repo.clone(), TARGET.into());
    fresh.publish(Some(&old), &new).unwrap();
    assert_eq!(scratch_ok(&repo, &["rev-parse", TARGET], b""), new);
}

#[test]
fn a_store_that_fails_while_staging_stops_every_later_publication() {
    let (_dir, repo, sink) = destination("sha1");
    let first = sink
        .raw(HistoryKind::Blob, b"first of a full batch", None, false)
        .unwrap();
    for n in 1..WRITE_BATCH {
        let bytes = n.to_string();
        sink.raw(HistoryKind::Blob, bytes.as_bytes(), None, false)
            .unwrap();
    }
    corrupt(&sink, &first);
    let next = sink.raw(HistoryKind::Blob, b"the next batch", None, false);
    assert!(next.is_err());
    let head = outside_commit(&repo, "stored by git itself");
    assert!(matches!(
        sink.publish(None, &head),
        Err(HistoryError::Unavailable(why)) if why.contains("earlier object store")
    ));
    let target = scratch(&repo, &["rev-parse", "--verify", "--quiet", TARGET], b"");
    assert!(!target.status.success(), "the target ref is never written");
}

#[test]
fn objects_are_staged_privately_inside_the_destination_git_directory() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, repo, sink) = destination("sha256");
    let (kind, _, bytes) = kinds().into_iter().next().unwrap();
    let id = sink.raw(kind, &bytes, None, false).unwrap();
    let file = sink.staged.lock().unwrap().file(&id).unwrap();
    let dir = file.parent().unwrap().to_path_buf();
    let git_dir = scratch_ok(&repo, &["rev-parse", "--absolute-git-dir"], b"");
    assert_eq!(dir.parent().unwrap(), std::path::Path::new(&git_dir));
    let name = dir.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("quecto-export-"), "{name}");
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    let head = outside_commit(&repo, "head");
    sink.publish(None, &head).unwrap();
    assert!(!dir.exists(), "a stored batch removes its directory");
}

#[test]
fn a_git_directory_git_cannot_read_back_as_a_line_is_named_in_the_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let odd = dir.path().join("line\nbreak.git");
    let path = odd.to_str().unwrap();
    scratch_ok(dir.path(), &["init", "-q", "--bare", path], b"");
    let sink = GitBoardHistorySink::new(odd.clone(), TARGET.into());
    let (kind, _, bytes) = kinds().into_iter().next().unwrap();
    assert!(
        sink.raw(kind, &bytes, None, true).is_ok(),
        "a dry run stages nothing"
    );
    match sink.raw(kind, &bytes, None, false) {
        Err(HistoryError::Unavailable(why)) => {
            assert!(why.contains("destination git directory"), "{why}");
            assert!(why.contains("line\\nbreak.git"), "{why}");
        }
        other => panic!("expected a refusal naming the directory: {other:?}"),
    }
}

#[test]
fn a_store_deadline_grows_with_its_batch_from_gits_own() {
    use std::time::Duration;
    let base = super::super::git::LOCAL_TIMEOUT;
    assert_eq!(batch_timeout(0, 0), base + Duration::from_secs(1));
    let full = batch_timeout(WRITE_BATCH, WRITE_BATCH_BYTES);
    let grown = Duration::from_millis(40_960) + Duration::from_secs(65);
    assert_eq!(full, base + grown);
    assert!(batch_timeout(1, 0) < batch_timeout(2, 0));
    assert!(batch_timeout(1, 0) < batch_timeout(1, 2 * 1024 * 1024));
    // Saturates rather than overflowing, however large a batch claims to be.
    assert!(batch_timeout(usize::MAX, usize::MAX) > full);
}
