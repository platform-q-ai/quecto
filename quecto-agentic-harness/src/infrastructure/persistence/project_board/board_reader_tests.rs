use super::*;
use std::cell::RefCell;

#[derive(Default)]
struct Flat {
    trees: BTreeMap<String, Vec<Entry>>,
    meta: Metadata,
    bytes: Content,
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
        Ok(self.bytes.clone())
    }
}
impl Flat {
    fn tree(&mut self, id: &str, rows: &[(&str, &str, &str)]) {
        self.meta.insert(
            id.into(),
            Found::Present(Object {
                kind: "tree".into(),
                size: rows.len() * 40,
                omitted: false,
            }),
        );
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
    fn blob(&mut self, id: &str, size: usize) {
        self.meta.insert(
            id.into(),
            Found::Present(Object {
                kind: "blob".into(),
                size,
                omitted: false,
            }),
        );
    }
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
    f.blob("a", 1);
    f.blob("link", 1);
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
    f.blob("p", 1);
    assert!(matches!(
        read(
            &f,
            "root",
            Limits {
                board_bytes: 1,
                ..LIMITS
            },
            &LeftOnRemote::default()
        ),
        Err(Unread::Failed(BoardStoreError::TooLarge(_)))
    ));
    assert!(f.fetched.borrow().is_empty());
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
        f.bytes = bytes
            .map(|bytes| BTreeMap::from([("a".into(), bytes)]))
            .unwrap_or_default();
        assert!(contents(&f, &[("a", 1)]).is_err());
    }
    f.bytes = BTreeMap::from([("wrong".into(), vec![0])]);
    assert!(contents(&f, &[("a", 1)]).is_err());
    f.bytes = BTreeMap::from([("a".into(), vec![0])]);
    assert!(contents(&f, &[("a", 1), ("a", 2)]).is_err());
    assert!(contents(&f, &[("a", 1), ("a", 1)]).is_ok());
    f.tree("root", &[("tasks", TREE, "tasks")]);
    f.tree(
        "tasks",
        &[("a.md", REGULAR_FILE, "a"), ("b.md", REGULAR_FILE, "a")],
    );
    f.blob("a", 1);
    let board = read(&f, "root", LIMITS, &LeftOnRemote::default()).unwrap();
    assert_eq!(board.quarantined.len(), 2);
    f.meta.insert(
        "root".into(),
        Found::Present(Object {
            kind: "tree".into(),
            size: LIMITS.tree_bytes + 1,
            omitted: false,
        }),
    );
    f.fetched.borrow_mut().clear();
    assert!(read(&f, "root", LIMITS, &LeftOnRemote::default()).is_err());
    assert!(f.fetched.borrow().is_empty());
}
