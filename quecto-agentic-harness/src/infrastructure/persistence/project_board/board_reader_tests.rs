use super::*;
use std::cell::RefCell;

#[derive(Default)]
struct Flat {
    trees: BTreeMap<String, Vec<Entry>>,
    meta: Metadata,
    bytes: Content,
    response: Option<Content>,
    fetched: RefCell<Vec<String>>,
}
impl Reader for Flat {
    fn metadata(&self, ids: &[&str]) -> Result<Metadata, BoardStoreError> {
        Ok(ids
            .iter()
            .map(|id| {
                (
                    (*id).into(),
                    self.meta.get(*id).cloned().unwrap_or(Found::Missing),
                )
            })
            .collect())
    }
    fn listing(&self, tree: &str, _: usize) -> Result<Vec<Entry>, Unread> {
        Ok(self.trees.get(tree).cloned().unwrap_or_default())
    }
    fn admitted(&self, wanted: &[(&str, usize)]) -> Result<Content, BoardStoreError> {
        self.fetched
            .borrow_mut()
            .extend(wanted.iter().map(|(id, _)| id.to_string()));
        Ok(self.response.clone().unwrap_or_else(|| {
            wanted
                .iter()
                .filter_map(|(id, _)| {
                    self.bytes
                        .get(*id)
                        .map(|bytes| ((*id).into(), bytes.clone()))
                })
                .collect()
        }))
    }
}
impl Flat {
    fn tree(&mut self, id: &str, rows: &[(&str, &str, &str)]) {
        self.object(id, "tree", rows.len() * 40);
        self.trees.insert(
            id.into(),
            rows.iter()
                .map(|(name, mode, oid)| Entry {
                    name: (*name).into(),
                    raw: name.as_bytes().to_vec(),
                    mode: (*mode).into(),
                    oid: (*oid).into(),
                })
                .collect(),
        );
    }
    fn object(&mut self, id: &str, kind: &str, size: usize) {
        self.meta.insert(
            id.into(),
            Found::Present(Object {
                kind: kind.into(),
                size,
                omitted: false,
            }),
        );
    }
}
fn refused_before_fetch(f: &Flat, limits: Limits) {
    f.fetched.borrow_mut().clear();
    assert!(matches!(
        read(f, "root", limits, &LeftOnRemote::default()),
        Err(Unread::Failed(BoardStoreError::TooLarge(_)))
    ));
    assert!(f.fetched.borrow().is_empty());
}
#[test]
fn flat_whole_board_admission_and_opaque_nonfetch() {
    let mut f = Flat::default();
    f.tree(
        "root",
        &[("tasks", TREE, "tasks"), ("archive", TREE, "archive")],
    );
    f.tree("archive", &[]);
    f.tree(
        "tasks",
        &[
            ("a.md", REGULAR_FILE, "a"),
            ("b.md", "120000", "link"),
            ("other", "160000", "commit"),
        ],
    );
    f.object("a", "blob", 1);
    f.object("link", "blob", 1);
    f.bytes.insert("a".into(), vec![0]);
    let board = read(&f, "root", LIMITS, &LeftOnRemote::default()).unwrap();
    assert_eq!(board.quarantined.len(), 3);
    assert_eq!(board.archive.as_deref(), Some("archive"));
    assert_eq!(*f.fetched.borrow(), ["a"]);
    f.fetched.borrow_mut().clear();
    f.tree(
        "root",
        &[("tasks", TREE, "tasks"), ("plans", TREE, "plans")],
    );
    f.tree("plans", &[("p.1.md", REGULAR_FILE, "p")]);
    f.object("p", "blob", 1);
    refused_before_fetch(
        &f,
        Limits {
            board_bytes: 1,
            ..LIMITS
        },
    );
    f.tree("plans", &[("p.1.md", REGULAR_FILE, "missing")]);
    let left = LeftOnRemote {
        trees: BTreeSet::from(["plans".into()]),
        ..LeftOnRemote::default()
    };
    let board = read(&f, "root", LIMITS, &left).unwrap();
    assert!(board.omitted.contains("missing"));
    assert_eq!(*f.fetched.borrow(), ["a"]);
}
#[test]
fn flat_callback_failures_and_shared_object() {
    let mut f = Flat::default();
    for bytes in [None, Some(vec![]), Some(vec![0, 0])] {
        f.response = Some(
            bytes
                .map(|bytes| BTreeMap::from([("a".into(), bytes)]))
                .unwrap_or_default(),
        );
        assert!(contents(&f, &[("a", 1)]).is_err());
    }
    f.response = Some(BTreeMap::from([("wrong".into(), vec![0])]));
    assert!(contents(&f, &[("a", 1)]).is_err());
    f.response = Some(BTreeMap::from([("a".into(), vec![0])]));
    assert!(contents(&f, &[("a", 1), ("a", 2)]).is_err());
    assert!(contents(&f, &[("a", 1), ("a", 1)]).is_ok());
    f.tree("root", &[("tasks", TREE, "tasks")]);
    f.tree(
        "tasks",
        &[("a.md", REGULAR_FILE, "a"), ("b.md", REGULAR_FILE, "a")],
    );
    f.object("a", "blob", 1);
    let board = read(&f, "root", LIMITS, &LeftOnRemote::default()).unwrap();
    assert_eq!(board.quarantined.len(), 2);
    f.object("root", "tree", LIMITS.tree_bytes + 1);
    refused_before_fetch(&f, LIMITS);
}

#[test]
fn flat_canonical_mixed_snapshot_matches_real_git() {
    use crate::application::project_board::ports::BoardHead;
    use crate::infrastructure::persistence::project_board::{
        artifacts::file_of, git::GitEnvironment,
    };
    use crate::infrastructure::test_support::board_artifacts::plan;
    let plan = plan("a", 1);
    let dir = tempfile::tempdir().unwrap();
    let git = Git::new(
        dir.path(),
        GitEnvironment::of_process(),
        plan.fields().proposed_by.clone(),
    )
    .unwrap();
    git.ok(&["init", "-q"], None).unwrap();
    let task = super::super::tests::a_task();
    let mut f = Flat::default();
    for (path, bytes) in [
        (
            "tasks/a.md",
            file_of(&BoardArtifact::Task(task.clone())).unwrap(),
        ),
        (
            "plans/a.1.md",
            file_of(&BoardArtifact::Plan(plan.clone())).unwrap(),
        ),
        ("tasks/old.json", vec![0]),
        ("tasks/b.md", vec![0]),
        ("tasks/b.json", vec![0]),
        ("archive/opaque", vec![0]),
    ] {
        if path.ends_with(".md") {
            let oid = git
                .text(&["hash-object", "-w", "--stdin"], Some(&bytes))
                .unwrap();
            f.bytes.insert(oid, bytes.clone());
        }
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    git.ok(&["add", "."], None).unwrap();
    let root = git.text(&["write-tree"], None).unwrap();
    let commit = git
        .text(&["commit-tree", &root], Some(b"fixture\n"))
        .unwrap();
    // Capture raw metadata/listings, not interpreted results. Archive bytes stay opaque.
    let mut trees = vec![root.clone()];
    while let Some(tree) = trees.pop() {
        let rows = git.listing(&tree, MAX_LISTING_BYTES).unwrap();
        let ids: Vec<_> = std::iter::once(tree.as_str())
            .chain(looked_up(&rows))
            .collect();
        f.meta.extend(git.metadata(&ids).unwrap());
        for row in &rows {
            if row.mode == TREE && matches!(row.name.as_str(), "tasks" | "plans") {
                trees.push(row.oid.clone());
            }
        }
        f.trees.insert(tree, rows);
    }
    let real = read_board(&git, &commit, LIMITS, &LeftOnRemote::default()).unwrap();
    let flat = read(&f, &root, LIMITS, &LeftOnRemote::default()).unwrap();
    let snapshot = flat.snapshot(BoardHead(commit.clone()));
    assert_eq!(snapshot.tasks, [task]);
    assert_eq!(snapshot.plans, [plan]);
    assert_eq!(snapshot, real.snapshot(BoardHead(commit)));
    assert_eq!(snapshot.quarantined.len(), 3);
    assert!(snapshot.quarantined.iter().all(|q| q.carried));
    assert_eq!(flat.kept, real.kept);
    assert_eq!(flat.folders, real.folders);
    assert_eq!(flat.archive.as_ref(), Some(real.archive.as_ref().unwrap()));
}
