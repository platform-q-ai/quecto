//! Clearing one file set aside in a folder besides `tasks/` (#2481 S10a):
//! the clear removes that file alone, by its object, whatever its mode,
//! and a folder it leaves empty is no folder.
use super::*;

/// The clear of the file set aside at `path` in bob's reading of the board
/// at `head`.
fn clear_of(repos: &Repos, path: &str) -> SetAsideClear {
    let board = store(&repos.bob).read().unwrap();
    let file = board.quarantined.iter().find(|file| file.path == path);
    let file = file.unwrap_or_else(|| panic!("{path} is set aside"));
    SetAsideClear {
        path: file.path.clone(),
        object: file.object.clone(),
        repeated: BTreeMap::new(),
    }
}

/// A gitlink beside a plan of its name is a stray twin: clearing it never
/// takes the plan, which is read on as it was.
#[test]
fn a_clear_of_a_stray_twin_keeps_the_file_of_its_name() {
    let repos = fresh();
    let head = board_of(&repos, |repo| {
        let plan = blob_in(repo, &bytes::<PlanCodec>(&plan("a", 1)));
        let rows = vec![
            ("100644", "a.1.md".into(), plan),
            ("160000", "a.1.md".into(), "c".repeat(40)),
        ];
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", Node::Made(rows)),
        ]
    });
    let clear = clear_of(&repos, "plans/a.1.md");
    let bob = store(&repos.bob);
    assert_eq!(
        clear.object,
        "c".repeat(40),
        "the gitlink is the one set aside"
    );
    let cleared = written(bob.clear_set_aside(&head, &clear, "board: clear the gitlink"));
    let remote = repos.remote();
    assert_eq!(
        at(&remote, &cleared, "plans/a.1.md"),
        at(&remote, &head, "plans/a.1.md")
    );
    let board = store(&repos.alice).read().unwrap();
    assert_eq!(board.plans, [plan("a", 1)]);
    assert!(board.quarantined.is_empty(), "{:?}", board.quarantined);
}

/// A clear removes a kept executable and a kept directory each alone, and
/// a clear that takes a folder's last file leaves no folder.
#[test]
fn a_clear_removes_a_kept_file_of_any_mode_and_an_emptied_folder() {
    let repos = fresh();
    let head = board_of(&repos, |repo| {
        let one = blob_in(repo, &bytes::<PlanCodec>(&plan("a", 1)));
        let three = blob_in(repo, &bytes::<PlanCodec>(&plan("a", 3)));
        let inner = raw_rows(repo, &[("100644", "f", blob_in(repo, b"f"))]);
        let plans = vec![
            ("100644", "a.1.md".into(), one),
            ("100755", "a.3.md".into(), three),
            ("40000", "a.4.md".into(), inner),
        ];
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", Node::Made(plans)),
            (
                "40000",
                "runs",
                Node::Tree(vec![regular("a.1.md", b"{".to_vec())]),
            ),
        ]
    });
    let bob = store(&repos.bob);
    let remote = repos.remote();
    let aside: Vec<String> = store(&repos.bob)
        .read()
        .unwrap()
        .quarantined
        .iter()
        .map(|file| format!("{} {}", file.path, file.carried))
        .collect();
    assert_eq!(
        aside,
        ["plans/a.3.md true", "plans/a.4.md true", "runs/a.1.md true"]
    );
    let mut at_head = head;
    for path in ["plans/a.3.md", "plans/a.4.md"] {
        let clear = clear_of(&repos, path);
        at_head = written(bob.clear_set_aside(&at_head, &clear, "board: clear a plan"));
    }
    assert_eq!(names(&remote, &at_head, "plans"), ["a.1.md"]);
    let clear = clear_of(&repos, "runs/a.1.md");
    let cleared = written(bob.clear_set_aside(&at_head, &clear, "board: clear run 1"));
    let root = sh(&remote, &["ls-tree", "--name-only", &cleared.0]);
    assert!(!root.lines().any(|name| name == "runs"), "{root}");
    let board = store(&repos.alice).read().unwrap();
    assert!(board.quarantined.is_empty(), "{:?}", board.quarantined);
    assert_eq!(board.plans, [plan("a", 1)]);
}
