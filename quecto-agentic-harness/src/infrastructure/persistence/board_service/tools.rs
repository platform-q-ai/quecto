//! SPIKE (board service): the importer from a `quecto/board` branch (full
//! history, every commit a revision), the export back to git (each commit
//! rebuilt byte for byte from the database), and the measurement tools.
use super::files::{BoardFile, Head};
use super::sqlite::{FileChange, RevisionMeta, SqliteBoard};
use crate::domain::project_board::entities::claim::Identity;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

pub(super) fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .cloned()
}

pub(super) fn number(args: &[String], name: &str, default: u64) -> u64 {
    flag(args, name)
        .and_then(|n| n.parse().ok())
        .unwrap_or(default)
}

pub fn run(args: &[String]) -> i32 {
    let outcome = match args.first().map(String::as_str) {
        Some("import") => import(args),
        Some("export") => export(args),
        Some("bench-latency") => super::bench::bench_latency(args),
        Some("bench-feed") => super::bench::bench_feed(args),
        Some("kill-test") => super::bench::kill_test(args),
        Some("backup") => super::bench::backup(args),
        Some("bench-contention-local") => super::bench::bench_contention_local(args),
        _ => Err("unknown tool".into()),
    };
    match outcome {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("board-server: {why}");
            1
        }
    }
}

fn git(dir: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .arg("--git-dir")
        .arg(dir)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        let bytes = bytes.to_vec();
        std::thread::spawn(move || stdin.write_all(&bytes));
    }
    let out = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    match out.status.success() {
        true => Ok(out.stdout),
        false => Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )),
    }
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/// Reads blobs through one `cat-file --batch`.
struct Blobs {
    child: std::process::Child,
    cache: HashMap<String, Vec<u8>>,
}

impl Blobs {
    fn new(dir: &Path) -> Result<Self, String> {
        let child = Command::new("git")
            .arg("--git-dir")
            .arg(dir)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            child,
            cache: HashMap::new(),
        })
    }

    fn get(&mut self, oid: &str) -> Result<Vec<u8>, String> {
        if let Some(bytes) = self.cache.get(oid) {
            return Ok(bytes.clone());
        }
        let stdin = self.child.stdin.as_mut().ok_or("no stdin")?;
        writeln!(stdin, "{oid}").map_err(|error| error.to_string())?;
        stdin.flush().map_err(|error| error.to_string())?;
        let stdout = self.child.stdout.as_mut().ok_or("no stdout")?;
        let mut header = vec![];
        let mut byte = [0u8];
        loop {
            stdout
                .read_exact(&mut byte)
                .map_err(|error| error.to_string())?;
            if byte[0] == b'\n' {
                break;
            }
            header.push(byte[0]);
        }
        let header = String::from_utf8_lossy(&header).to_string();
        let size: usize = header
            .split(' ')
            .nth(2)
            .and_then(|size| size.parse().ok())
            .ok_or_else(|| format!("cat-file said {header:?}"))?;
        let mut bytes = vec![0; size + 1];
        stdout
            .read_exact(&mut bytes)
            .map_err(|error| error.to_string())?;
        bytes.truncate(size);
        self.cache.insert(oid.to_string(), bytes.clone());
        Ok(bytes)
    }
}

/// Mode marking an empty directory (a tree with nothing under it).
const EMPTY_DIR: &str = "040000";

/// Every file of commit `commit`, by path: blobs (and links) with their
/// bytes, a gitlink with its id as text, an empty tree as itself.
fn tree_files(
    dir: &Path,
    commit: &str,
    blobs: &mut Blobs,
) -> Result<BTreeMap<String, (String, Vec<u8>)>, String> {
    let listing = git(
        dir,
        &["ls-tree", "-r", "-t", "-z", "--full-tree", commit],
        None,
    )?;
    let mut files = BTreeMap::new();
    let mut trees = BTreeSet::new();
    for entry in listing.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let entry = String::from_utf8(entry.to_vec()).map_err(|_| "a non-UTF-8 path")?;
        let (meta, path) = entry.split_once('\t').ok_or("a malformed ls-tree row")?;
        let mut parts = meta.split(' ');
        let (mode, kind, oid) = (
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        match kind {
            "blob" => {
                files.insert(path.to_string(), (mode.to_string(), blobs.get(oid)?));
            }
            "tree" => {
                trees.insert(path.to_string());
            }
            _ => {
                files.insert(
                    path.to_string(),
                    (mode.to_string(), oid.as_bytes().to_vec()),
                );
            }
        }
    }
    for tree in trees {
        let prefix = format!("{tree}/");
        if !files.keys().any(|path| path.starts_with(&prefix)) {
            files.insert(tree, (EMPTY_DIR.to_string(), vec![]));
        }
    }
    Ok(files)
}

/// A raw commit object's header and message without its `tree` and
/// `parent` lines: what the export puts back after them.
fn commit_meta(raw: &[u8]) -> (Vec<u8>, Identity, String) {
    let split = raw
        .windows(2)
        .position(|w| w == b"\n\n")
        .map_or(raw.len(), |at| at + 1);
    let (header, message) = raw.split_at(split);
    let mut kept = vec![];
    let mut writer = Identity {
        name: "unknown".into(),
        email: "unknown@quecto.invalid".into(),
    };
    for line in header.split_inclusive(|b| *b == b'\n') {
        if line.starts_with(b"tree ") || line.starts_with(b"parent ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix(b"committer ") {
            let rest = String::from_utf8_lossy(rest);
            if let Some((name, after)) = rest.split_once(" <") {
                writer = Identity {
                    name: name.to_string(),
                    email: after
                        .split_once('>')
                        .map_or("", |(email, _)| email)
                        .to_string(),
                };
            }
        }
        kept.extend_from_slice(line);
    }
    kept.extend_from_slice(message);
    let message = String::from_utf8_lossy(message);
    let subject = message
        .trim_start_matches('\n')
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    (kept, writer, subject)
}

/// `import --db <file> --git-dir <dir> --ref <ref>`: every commit of the
/// board branch, oldest first, as a revision of epoch 1.
fn import(args: &[String]) -> Result<(), String> {
    let started = Instant::now();
    let db = PathBuf::from(flag(args, "--db").ok_or("--db")?);
    let dir = PathBuf::from(flag(args, "--git-dir").ok_or("--git-dir")?);
    let reference = flag(args, "--ref").unwrap_or_else(|| "refs/heads/quecto/board".into());
    let board = SqliteBoard::open(&db).map_err(|error| format!("{error:?}"))?;
    if board
        .head()
        .map_err(|error| format!("{error:?}"))?
        .is_some()
    {
        return Err("the database already holds a board".into());
    }
    let listing = text(git(
        &dir,
        &[
            "rev-list",
            "--reverse",
            "--topo-order",
            "--parents",
            &reference,
        ],
        None,
    )?);
    let mut blobs = Blobs::new(&dir)?;
    let mut previous: Option<String> = None;
    let mut before: BTreeMap<String, (String, Vec<u8>)> = BTreeMap::new();
    let mut count = 0u64;
    for line in listing.lines() {
        let mut ids = line.split(' ');
        let commit = ids.next().ok_or("an empty rev-list row")?.to_string();
        let parents: Vec<&str> = ids.collect();
        match (parents.as_slice(), &previous) {
            ([], None) => {}
            ([parent], Some(previous)) if parent == previous => {}
            _ => {
                return Err(format!(
                    "{commit}: the board's history is not linear ({parents:?})"
                ));
            }
        }
        let raw = git(&dir, &["cat-file", "commit", &commit], None)?;
        let (meta, writer, subject) = commit_meta(&raw);
        let files = tree_files(&dir, &commit, &mut blobs)?;
        let mut changes = vec![];
        for (path, (mode, bytes)) in &files {
            if before.get(path) != Some(&(mode.clone(), bytes.clone())) {
                changes.push(FileChange {
                    path: path.clone(),
                    mode: mode.clone(),
                    bytes: Some(bytes.clone()),
                });
            }
        }
        for (path, (mode, _)) in &before {
            if !files.contains_key(path) {
                changes.push(FileChange {
                    path: path.clone(),
                    mode: mode.clone(),
                    bytes: None,
                });
            }
        }
        count += 1;
        let at = Head {
            epoch: 1,
            revision: count,
        };
        let revision_meta = RevisionMeta {
            git_commit: Some(commit.clone()),
            git_meta: Some(meta),
            ..RevisionMeta::by(
                writer,
                if subject.is_empty() {
                    "(no message)"
                } else {
                    &subject
                },
                "import",
            )
        };
        board
            .append_raw(at, &changes, &revision_meta)
            .map_err(|error| format!("{error:?}"))?;
        before = files;
        previous = Some(commit);
    }
    let current = board.current().map_err(|error| format!("{error:?}"))?;
    println!(
        "{}",
        serde_json::json!({
            "imported_revisions": count,
            "head": current.head.text(),
            "tip": previous,
            "tasks": current.snapshot.tasks.len(),
            "plans": current.snapshot.plans.len(),
            "runs": current.snapshot.runs.len(),
            "changes": current.snapshot.changes.len(),
            "reviews": current.snapshot.reviews.len(),
            "discussions": current.snapshot.discussions.len(),
            "set_aside": current.snapshot.quarantined.len(),
            "files": current.files.len(),
            "ms": started.elapsed().as_millis(),
        })
    );
    Ok(())
}

/// Builds the tree of `files` (nested), returning its id.
fn build_tree(
    dir: &Path,
    files: &[BoardFile],
    blob_ids: &mut HashMap<Vec<u8>, String>,
) -> Result<String, String> {
    #[derive(Default)]
    struct Node {
        files: BTreeMap<String, (String, String)>,
        dirs: BTreeMap<String, Node>,
        empty: bool,
    }
    let mut root = Node::default();
    for file in files {
        let parts: Vec<&str> = file.path.split('/').collect();
        let (name, folders) = parts.split_last().ok_or("an empty path")?;
        let mut node = &mut root;
        for folder in folders {
            node = node.dirs.entry((*folder).to_string()).or_default();
        }
        if file.mode == EMPTY_DIR {
            node.dirs.entry((*name).to_string()).or_default().empty = true;
            continue;
        }
        let (kind, oid) = match file.mode.as_str() {
            "160000" => ("commit", String::from_utf8_lossy(&file.bytes).to_string()),
            _ => {
                let oid = match blob_ids.get(&file.bytes) {
                    Some(oid) => oid.clone(),
                    None => {
                        let oid = text(git(
                            dir,
                            &["hash-object", "-w", "--stdin"],
                            Some(&file.bytes),
                        )?);
                        blob_ids.insert(file.bytes.clone(), oid.clone());
                        oid
                    }
                };
                ("blob", oid)
            }
        };
        node.files.insert(
            (*name).to_string(),
            (file.mode.clone(), format!("{kind} {oid}")),
        );
    }
    fn write(dir: &Path, node: &Node) -> Result<String, String> {
        let mut listing = String::new();
        for (name, (mode, entry)) in &node.files {
            listing.push_str(&format!("{mode} {entry}\t{name}\0"));
        }
        for (name, child) in &node.dirs {
            let oid = write(dir, child)?;
            listing.push_str(&format!("040000 tree {oid}\t{name}\0"));
        }
        let _ = node.empty;
        Ok(text(git(
            dir,
            &["mktree", "-z", "--missing"],
            Some(listing.as_bytes()),
        )?))
    }
    write(dir, &root)
}

/// `export --db <file> --git-dir <dir> --ref <out ref> [--format4]`: every
/// revision as a commit, imported ones rebuilt from their recorded header,
/// and how many come out as the very commit they were.
fn export(args: &[String]) -> Result<(), String> {
    let started = Instant::now();
    let db = PathBuf::from(flag(args, "--db").ok_or("--db")?);
    let dir = PathBuf::from(flag(args, "--git-dir").ok_or("--git-dir")?);
    let reference = flag(args, "--ref").ok_or("--ref")?;
    let board = SqliteBoard::open(&db).map_err(|error| format!("{error:?}"))?;
    let head = board
        .head()
        .map_err(|error| format!("{error:?}"))?
        .ok_or("no board")?;
    let mut blob_ids = HashMap::new();
    let mut parent: Option<String> = None;
    let (mut same, mut differ, mut native) = (0u64, vec![], 0u64);
    for revision in 1..=head.revision {
        let (row, _) = board
            .revision(revision)
            .map_err(|error| format!("{error:?}"))?;
        let mut files = board
            .files_at(revision)
            .map_err(|error| format!("{error:?}"))?;
        if args.iter().any(|arg| arg == "--format4") && revision == head.revision {
            for file in &mut files {
                if file.path == super::files::format_path() {
                    file.bytes = b"quecto-board 4\n".to_vec();
                }
            }
        }
        let tree = build_tree(&dir, &files, &mut blob_ids)?;
        let mut raw = format!("tree {tree}\n").into_bytes();
        if let Some(parent) = &parent {
            raw.extend_from_slice(format!("parent {parent}\n").as_bytes());
        }
        match &row.git_meta {
            Some(meta) => raw.extend_from_slice(meta),
            None => {
                native += 1;
                let when = humantime::parse_rfc3339_weak(&row.server_time)
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_secs());
                let who = format!("{} <{}> {when} +0000", row.writer_name, row.writer_email);
                raw.extend_from_slice(
                    format!("author {who}\ncommitter {who}\n\n{}\n", row.message).as_bytes(),
                );
            }
        }
        let commit = text(git(
            &dir,
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            Some(&raw),
        )?);
        match &row.git_commit {
            Some(original) if *original == commit => same += 1,
            Some(original) => differ.push(format!("r{revision}: {original} -> {commit}")),
            None => {}
        }
        parent = Some(commit);
    }
    let tip = parent.ok_or("no revision")?;
    git(&dir, &["update-ref", &reference, &tip], None)?;
    println!(
        "{}",
        serde_json::json!({
            "exported_revisions": head.revision,
            "identical_commits": same,
            "differing_commits": differ,
            "native_revisions": native,
            "tip": tip,
            "ms": started.elapsed().as_millis(),
        })
    );
    Ok(())
}
