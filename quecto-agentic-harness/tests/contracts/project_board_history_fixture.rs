//! Scratch Git fixture oracle only, not a production history adapter/interpreter.
use super::project_board_store::task;
use quecto::application::project_board::ports::{history::*, *};
use quecto::composition::bootstrap::project_board_history::{
    BoardHistoryEnvironment, BoardHistoryHandles, build_board_history,
};
use quecto::domain::project_board::{entities::claim::Identity, value_objects::telemetry::BoardId};
use quecto::infrastructure::persistence::project_board::{
    git::{Git, GitEnvironment},
    store::GitBoardStore,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Corpus {
    pub algorithm: HistoryAlgorithm,
    pub source_ref: String,
    pub source_remote: String,
    pub source_head: String,
    pub revisions: Vec<HistoryRevision>,
    pub objects: Vec<HistoryObject>,
}
pub const BOARD: &str = "refs/heads/quecto/board";
pub const TARGET: &str = "refs/heads/quecto/rollback";
pub fn writer() -> Identity {
    Identity {
        name: "History".into(),
        email: "history@example.com".into(),
    }
}

pub fn git(repo: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut command = Command::new("timeout");
    command
        .args(["-s", "KILL", "60", "git"])
        .args(args)
        .current_dir(repo)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", repo)
        .envs([
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_AUTHOR_NAME", "History"),
            ("GIT_AUTHOR_EMAIL", "history@example.com"),
            ("GIT_COMMITTER_NAME", "History"),
            ("GIT_COMMITTER_EMAIL", "history@example.com"),
            ("GIT_AUTHOR_DATE", "1700000000 +0000"),
            ("GIT_COMMITTER_DATE", "1700000000 +0000"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: the pre-exec hook only calls the async-signal-safe setrlimit.
        unsafe {
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::setrlimit(libc::RLIMIT_CORE, &limit) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
    }
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(
        output.status.success(),
        "fixture git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
pub fn text(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repo, args, b""))
        .unwrap()
        .trim_end()
        .to_string()
}
pub fn hash(repo: &Path, kind: &str, bytes: &[u8]) -> String {
    String::from_utf8(git(
        repo,
        &["hash-object", "--literally", "-w", "-t", kind, "--stdin"],
        bytes,
    ))
    .unwrap()
    .trim()
    .into()
}
fn binary(oid: &str) -> Vec<u8> {
    oid.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
pub fn raw_tree(repo: &Path, entries: &[(&str, &[u8], &str)]) -> String {
    let mut bytes = vec![];
    for (mode, name, oid) in entries {
        bytes.extend_from_slice(mode.as_bytes());
        bytes.push(b' ');
        bytes.extend_from_slice(name);
        bytes.push(0);
        bytes.extend(binary(oid));
    }
    hash(repo, "tree", &bytes)
}
pub fn commit(repo: &Path, tree: &str, parents: &[&str], extra: &str, message: &str) -> String {
    let mut bytes = format!("tree {tree}\n");
    for parent in parents {
        bytes.push_str(&format!("parent {parent}\n"));
    }
    bytes.push_str("author History <history@example.com> 1700000000 +0530\ncommitter History <history@example.com> 1700000001 -0230\n");
    bytes.push_str(extra);
    bytes.push('\n');
    bytes.push_str(message);
    hash(repo, "commit", bytes.as_bytes())
}
pub fn advance(repo: &Path, tree: &str, extra: &str, message: &str) -> String {
    let old = text(repo, &["rev-parse", BOARD]);
    let next = commit(repo, tree, &[&old], extra, message);
    git(repo, &["update-ref", BOARD, &next, &old], b"");
    next
}
fn entries(bytes: &[u8], width: usize) -> Vec<HistoryEntry> {
    let mut rows = vec![];
    let mut cursor = 0;
    while cursor < bytes.len() {
        let space = bytes[cursor..].iter().position(|b| *b == b' ').unwrap() + cursor;
        let nul = bytes[space + 1..].iter().position(|b| *b == 0).unwrap() + space + 1;
        let mode =
            u32::from_str_radix(std::str::from_utf8(&bytes[cursor..space]).unwrap(), 8).unwrap();
        let oid = bytes[nul + 1..nul + 1 + width]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        rows.push(HistoryEntry {
            mode: format!("{mode:06o}"),
            name: bytes[space + 1..nul].to_vec(),
            oid,
        });
        cursor = nul + 1 + width;
    }
    rows
}
fn closure(repo: &Path, oid: &str, width: usize, objects: &mut BTreeMap<String, HistoryObject>) {
    if objects.contains_key(oid) {
        return;
    }
    let kind = text(repo, &["cat-file", "-t", oid]);
    let body = git(repo, &["cat-file", &kind, oid], b"");
    let typed = match kind.as_str() {
        "blob" => HistoryKind::Blob,
        "tree" => HistoryKind::Tree,
        _ => panic!("unexpected fixture closure kind {kind}"),
    };
    let rows = if typed == HistoryKind::Tree {
        entries(&body, width)
    } else {
        vec![]
    };
    objects.insert(
        oid.into(),
        HistoryObject {
            oid: oid.into(),
            kind: typed,
            bytes: body,
            entries: rows.clone(),
        },
    );
    for row in rows {
        if row.mode != "160000" {
            closure(repo, &row.oid, width, objects);
        }
    }
}
pub fn manifest(repo: &Path, algorithm: HistoryAlgorithm) -> Corpus {
    let ids: Vec<_> = text(repo, &["rev-list", "--reverse", BOARD])
        .lines()
        .map(str::to_owned)
        .collect();
    let width = match algorithm {
        HistoryAlgorithm::Sha1 => 20,
        HistoryAlgorithm::Sha256 => 32,
    };
    let mut objects = BTreeMap::new();
    let mut revisions = vec![];
    for (index, id) in ids.iter().enumerate() {
        let raw = git(repo, &["cat-file", "commit", id], b"");
        let tree =
            String::from_utf8(raw.split(|b| *b == b'\n').next().unwrap()[5..].to_vec()).unwrap();
        closure(repo, &tree, width, &mut objects);
        revisions.push(HistoryRevision {
            sequence: index as u64 + 1,
            parent: if index == 0 { None } else { Some(index as u64) },
            root: tree,
            writer: writer(),
            message: "original raw body is authoritative".into(),
            at: 1700000000,
            original: Some(HistoryObject {
                oid: id.clone(),
                kind: HistoryKind::Commit,
                bytes: raw,
                entries: vec![],
            }),
        });
    }
    Corpus {
        algorithm,
        source_ref: BOARD.into(),
        source_remote: "origin".into(),
        source_head: ids.last().unwrap().clone(),
        revisions,
        objects: objects.into_values().collect(),
    }
}

pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub repo: PathBuf,
    pub destination: PathBuf,
    pub database: PathBuf,
    pub history: Corpus,
    pub set_aside_head: String,
    pub clear_head: String,
    pub repeated_head: String,
    pub repeated_seen: BTreeSet<String>,
    pub repeated_keep: String,
}
impl Fixture {
    pub fn new(algorithm: HistoryAlgorithm, count: usize) -> Self {
        assert!(count >= 12);
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path();
        let repo = base.join("source.git");
        let destination = base.join("destination.git");
        let clone = base.join("writer");
        let format = match algorithm {
            HistoryAlgorithm::Sha1 => "--object-format=sha1",
            HistoryAlgorithm::Sha256 => "--object-format=sha256",
        };
        git(
            base,
            &["init", "-q", "--bare", format, repo.to_str().unwrap()],
            b"",
        );
        git(
            base,
            &[
                "init",
                "-q",
                "--bare",
                format,
                destination.to_str().unwrap(),
            ],
            b"",
        );
        git(
            &repo,
            &["remote", "add", "origin", repo.to_str().unwrap()],
            b"",
        );
        git(base, &["init", "-q", format, clone.to_str().unwrap()], b"");
        git(
            &clone,
            &["remote", "add", "origin", repo.to_str().unwrap()],
            b"",
        );
        let vars: Vec<(OsString, OsString)> = vec![
            ("PATH".into(), std::env::var_os("PATH").unwrap()),
            ("HOME".into(), base.into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        let store = GitBoardStore::new(
            Git::new(&clone, GitEnvironment::allowlisted(vars), writer()).unwrap(),
            "origin",
        )
        .unwrap();
        store.bootstrap().unwrap();
        for (id, title) in [("a", "First"), ("b", "To be set aside")] {
            let head = store.read().unwrap().head;
            assert!(matches!(
                store.write_tasks(&head, &[task(id, title)], "board: fixture"),
                Ok(BoardWrite::Written(_))
            ));
        }
        let old = text(&repo, &["rev-parse", BOARD]);
        let root = text(&repo, &["rev-parse", &format!("{old}^{{tree}}")]);
        let tasks = text(&repo, &["rev-parse", &format!("{old}:tasks")]);
        let a = text(&repo, &["rev-parse", &format!("{old}:tasks/a.md")]);
        assert!(!tasks.is_empty());
        let broken = hash(&repo, "blob", b"{");
        let changed = raw_tree(
            &repo,
            &[("100644", b"a.md", &a), ("100644", b"b.md", &broken)],
        );
        let mut listing = String::from_utf8(git(&repo, &["ls-tree", &root], b"")).unwrap();
        listing = listing
            .lines()
            .map(|line| {
                if line.ends_with("\ttasks") {
                    format!("040000 tree {changed}\ttasks\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        let root = String::from_utf8(git(&repo, &["mktree"], listing.as_bytes()))
            .unwrap()
            .trim()
            .to_string();
        let set_aside_head = advance(&repo, &root, "", "set aside exact broken bytes\n");
        let snapshot = store.read().unwrap();
        assert_eq!(snapshot.quarantined.len(), 1);
        let q = &snapshot.quarantined[0];
        let clear = SetAsideClear {
            path: q.path.clone(),
            object: q.object.clone(),
            repeated: Default::default(),
        };
        assert!(matches!(
            store.clear_set_aside(&snapshot.head, &clear, "board: actual clear"),
            Ok(BoardWrite::Written(_))
        ));
        assert!(store.read().unwrap().quarantined.is_empty());
        let clear_head = text(&repo, &["rev-parse", BOARD]);
        let repeated_keep = text(&repo, &["rev-parse", &format!("{BOARD}:tasks/a.md")]);
        let other = hash(&repo, "blob", b"{another duplicate copy");
        let repeated_tasks = raw_tree(
            &repo,
            &[
                ("100644", b"a.md", &repeated_keep),
                ("100644", b"a.md", &other),
                ("100644", b"z.md", &broken),
            ],
        );
        let listing = String::from_utf8(git(&repo, &["ls-tree", &root], b"")).unwrap();
        let listing: String = listing
            .lines()
            .map(|line| {
                if line.ends_with("\ttasks") {
                    format!("040000 tree {repeated_tasks}\ttasks\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        let repeated_root = String::from_utf8(git(&repo, &["mktree"], listing.as_bytes()))
            .unwrap()
            .trim()
            .to_string();
        let repeated_head = advance(&repo, &repeated_root, "", "actual repeated-name tokens\n");
        let snapshot = store.read().unwrap();
        let repeated_seen: BTreeSet<_> = snapshot
            .quarantined
            .iter()
            .filter(|q| q.path == "tasks/a.md")
            .map(|q| q.object.clone())
            .collect();
        assert_eq!(repeated_seen.len(), 2);
        assert!(repeated_seen.contains(&repeated_keep));
        let q = snapshot
            .quarantined
            .iter()
            .find(|q| q.path == "tasks/z.md")
            .unwrap();
        let clear = SetAsideClear {
            path: q.path.clone(),
            object: q.object.clone(),
            repeated: BTreeMap::from([(
                "tasks/a.md".into(),
                KeepCopy::Object {
                    keep: repeated_keep.clone(),
                    seen: repeated_seen.clone(),
                },
            )]),
        };
        assert!(matches!(
            store.clear_set_aside(&snapshot.head, &clear, "board: actual repeated seen keep"),
            Ok(BoardWrite::Written(_))
        ));
        assert!(store.read().unwrap().quarantined.is_empty());
        for n in 7..11 {
            let head = store.read().unwrap().head;
            assert!(matches!(
                store.write_tasks(
                    &head,
                    &[task("a", &format!("Owner fixture {n}"))],
                    "board: owner twelve"
                ),
                Ok(BoardWrite::Written(_))
            ));
        }
        let binary = hash(&repo, "blob", b"opaque\0\xffarchive\n");
        let nested = raw_tree(
            &repo,
            &[
                ("000100644", b"dupe", &binary),
                ("100644", b"dupe", &binary),
                ("100755", b"bad\xff\tname", &binary),
            ],
        );
        let foreign = "1".repeat(match algorithm {
            HistoryAlgorithm::Sha1 => 40,
            HistoryAlgorithm::Sha256 => 64,
        });
        let archive = raw_tree(
            &repo,
            &[
                ("100644", b"file-mode-actual-tree", &nested),
                ("40000", b"tree-mode-actual-blob", &binary),
                ("160000", b"foreign", &foreign),
            ],
        );
        let head = text(&repo, &["rev-parse", BOARD]);
        let root = text(&repo, &["rev-parse", &format!("{head}^{{tree}}")]);
        let mut listing = String::from_utf8(git(&repo, &["ls-tree", &root], b"")).unwrap();
        listing.push_str(&format!("040000 tree {archive}\tarchive\n"));
        let root = String::from_utf8(git(&repo, &["mktree"], listing.as_bytes()))
            .unwrap()
            .trim()
            .to_string();
        advance(
            &repo,
            &root,
            "encoding ISO-8859-1\ngpgsig -----BEGIN SIGNATURE-----\n fixture signature bytes\n -----END SIGNATURE-----\nx-extra preserve-order\n",
            "signed raw history\n",
        );
        for n in 12..count {
            advance(&repo, &root, "", &format!("scratch corpus {n}\n"));
        }
        let history = manifest(&repo, algorithm);
        assert_eq!(history.revisions.len(), count);
        let database = base.join("board.sqlite");
        Self {
            directory,
            repo,
            destination,
            database,
            history,
            set_aside_head,
            clear_head,
            repeated_head,
            repeated_seen,
            repeated_keep,
        }
    }
    pub fn handles(&self) -> BoardHistoryHandles {
        build_board_history(BoardHistoryEnvironment {
            destination_repo: self.destination.clone(),
            database: self.database.clone(),
            target_ref: TARGET.into(),
            board: BoardId::of_hex("0123456789abcdef").unwrap(),
            writer: writer(),
        })
    }
    pub fn verify(&self) {
        let width = match self.history.algorithm {
            HistoryAlgorithm::Sha1 => 40,
            HistoryAlgorithm::Sha256 => 64,
        };
        let mut previous = None;
        for revision in &self.history.revisions {
            let original = revision.original.as_ref().unwrap();
            assert_eq!(original.oid.len(), width);
            assert_eq!(
                git(&self.repo, &["cat-file", "commit", &original.oid], b""),
                original.bytes
            );
            assert_eq!(
                String::from_utf8(git(
                    &self.repo,
                    &["hash-object", "--literally", "-t", "commit", "--stdin"],
                    &original.bytes
                ))
                .unwrap()
                .trim(),
                original.oid
            );
            let parents: Vec<_> = String::from_utf8(original.bytes.clone())
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("parent ").map(str::to_owned))
                .collect();
            assert_eq!(parents, previous.iter().cloned().collect::<Vec<_>>());
            previous = Some(original.oid.clone());
        }
        assert_eq!(previous.as_deref(), Some(self.history.source_head.as_str()));
        for object in &self.history.objects {
            let kind = match object.kind {
                HistoryKind::Blob => "blob",
                HistoryKind::Tree => "tree",
                HistoryKind::Commit => panic!("commits have separate raw original rows"),
            };
            assert_eq!(text(&self.repo, &["cat-file", "-t", &object.oid]), kind);
            assert_eq!(
                git(&self.repo, &["cat-file", kind, &object.oid], b""),
                object.bytes
            );
            assert_eq!(
                String::from_utf8(git(
                    &self.repo,
                    &["hash-object", "--literally", "-t", kind, "--stdin"],
                    &object.bytes
                ))
                .unwrap()
                .trim(),
                object.oid,
                "full original object body hash"
            );
        }
        for (name, expected) in [
            (b"file-mode-actual-tree".as_slice(), HistoryKind::Tree),
            (b"tree-mode-actual-blob".as_slice(), HistoryKind::Blob),
        ] {
            let row = self
                .history
                .objects
                .iter()
                .flat_map(|o| &o.entries)
                .find(|r| r.name == name)
                .unwrap();
            assert_eq!(
                self.history
                    .objects
                    .iter()
                    .find(|o| o.oid == row.oid)
                    .unwrap()
                    .kind,
                expected,
                "fixture exercises actual kind despite declared mode"
            );
        }
        let setaside = format!("{}:tasks/b.md", self.set_aside_head);
        assert_eq!(git(&self.repo, &["show", &setaside], b""), b"{");
        assert!(
            text(&self.repo, &["ls-tree", "-r", &self.clear_head])
                .lines()
                .all(|line| !line.ends_with("\ttasks/b.md"))
        );
        let duplicate_tree = text(
            &self.repo,
            &["rev-parse", &format!("{}:tasks", self.repeated_head)],
        );
        let duplicate = self
            .history
            .objects
            .iter()
            .find(|o| o.oid == duplicate_tree)
            .unwrap();
        assert_eq!(
            duplicate
                .entries
                .iter()
                .filter(|r| r.name == b"a.md")
                .map(|r| r.oid.clone())
                .collect::<BTreeSet<_>>(),
            self.repeated_seen
        );
        let resolved = self.history.revisions[6]
            .original
            .as_ref()
            .unwrap()
            .oid
            .clone();
        assert_eq!(
            text(
                &self.repo,
                &["rev-parse", &format!("{resolved}:tasks/a.md")]
            ),
            self.repeated_keep
        );
        let all: BTreeSet<_> = self
            .history
            .objects
            .iter()
            .map(|o| o.oid.as_str())
            .collect();
        assert!(
            self.history
                .objects
                .iter()
                .any(|o| o.bytes == b"opaque\0\xffarchive\n")
        );
        assert!(
            self.history
                .objects
                .iter()
                .any(|o| o.bytes.windows(9).any(|b| b == b"000100644"))
        );
        assert!(self.history.revisions.iter().any(|r| {
            r.original
                .as_ref()
                .unwrap()
                .bytes
                .windows(8)
                .any(|b| b == b"gpgsig -")
        }));
        for object in &self.history.objects {
            for row in &object.entries {
                if row.mode == "160000" {
                    assert!(
                        !all.contains(row.oid.as_str()),
                        "foreign gitlink is not fetched"
                    );
                } else {
                    assert!(
                        all.contains(row.oid.as_str()),
                        "actual-kind closure complete"
                    );
                }
            }
        }
    }
}

pub fn fingerprint(path: &Path) -> Vec<(String, Vec<Vec<String>>)> {
    let c = rusqlite::Connection::open(path).unwrap();
    let names:Vec<String>=c.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut statement = c
                .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid"))
                .unwrap();
            let cols = statement.column_count();
            let rows = statement
                .query_map([], |r| {
                    Ok((0..cols)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect())
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

impl Fixture {
    pub fn epoch_store(
        &self,
    ) -> quecto::infrastructure::persistence::sqlite::project_board::history::SqliteBoardHistoryStore
    {
        quecto::infrastructure::persistence::sqlite::project_board::history::SqliteBoardHistoryStore::new(self.database.clone(), writer(), BoardId::of_hex("0123456789abcdef").unwrap())
    }
}
