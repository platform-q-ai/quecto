//! Clearing files set aside, over a real remote: a name listed twice is
//! cleared by naming the copy to keep, and names are cleared one at a
//! time, another repeated name carried forward exactly as it was.
use super::*;

/// The entries of the board's `tasks/` tree at `head` on the remote, as
/// `name oid` lines in tree order.
fn listing(remote: &Path, head: &BoardHead) -> Vec<String> {
    sh(remote, &["ls-tree", &format!("{}:tasks", head.0)])
        .lines()
        .map(|row| {
            let (meta, name) = row.split_once('\t').unwrap();
            format!("{name} {}", meta.split(' ').nth(2).unwrap())
        })
        .collect()
}

fn other(id: &str) -> Task {
    let mut fields = task(id).into_fields();
    fields.title = "Another".into();
    Task::new(fields).unwrap()
}

fn oid_of(repo: &Path, task: &Task) -> String {
    sh_with(repo, &["hash-object", "--stdin"], &encode(task).unwrap())
}

#[test]
fn a_name_listed_twice_is_cleared_by_naming_the_copy_to_keep() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    raw_board(
        &repos.remote(),
        &[
            (
                "100644",
                "dup.json",
                Object::Blob(encode(&task("dup")).unwrap()),
            ),
            (
                "100644",
                "dup.json",
                Object::Blob(encode(&other("dup")).unwrap()),
            ),
            (
                "100644",
                "ok.json",
                Object::Blob(encode(&task("ok")).unwrap()),
            ),
        ],
        &[&repos.bob],
    );
    let bob = store_with(&repos.bob, vars(&repos.bob, &[]));
    let head = bob.read().unwrap().head;
    let (first, second) = (
        oid_of(&repos.bob, &task("dup")),
        oid_of(&repos.bob, &other("dup")),
    );
    let refused = |keep: Option<&str>| match bob.clear_set_aside(
        &head,
        "tasks/dup.json",
        keep,
        "board: clear dup",
    ) {
        Ok(BoardWrite::Refused(why)) => why,
        other => panic!("{keep:?}: {other:?}"),
    };
    let why = refused(None);
    assert!(
        why.contains(&first) && why.contains(&second),
        "both copies named: {why}"
    );
    refused(Some(&"0".repeat(first.len())));
    // A file the board holds, but not one of this name's copies.
    refused(Some(&oid_of(&repos.bob, &task("ok"))));
    let cleared =
        written(bob.clear_set_aside(&head, "tasks/dup.json", Some(&second), "board: clear dup"));
    let after = bob.read().unwrap();
    assert_eq!(after.quarantined, vec![]);
    assert!(
        after.tasks.contains(&other("dup")),
        "the copy kept is the task now"
    );
    let dup: Vec<String> = listing(&repos.remote(), &cleared)
        .into_iter()
        .filter(|row| row.starts_with("dup.json"))
        .collect();
    assert_eq!(dup, [format!("dup.json {second}")]);
    assert_eq!(
        bob.committed_by(&cleared.0).unwrap(),
        bob.writer(),
        "the board records who cleared it"
    );
}

#[test]
fn names_listed_twice_are_cleared_one_at_a_time_and_the_other_is_carried_as_it_was() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    raw_board(
        &repos.remote(),
        &[
            (
                "100644",
                "dup.json",
                Object::Blob(encode(&task("dup")).unwrap()),
            ),
            (
                "100644",
                "dup.json",
                Object::Blob(encode(&other("dup")).unwrap()),
            ),
            (
                "100644",
                "two.json",
                Object::Blob(encode(&task("two")).unwrap()),
            ),
            (
                "100644",
                "two.json",
                Object::Blob(encode(&other("two")).unwrap()),
            ),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let head = bob.read().unwrap().head;
    let keep = oid_of(&repos.bob, &task("dup"));
    let cleared =
        written(bob.clear_set_aside(&head, "tasks/dup.json", Some(&keep), "board: clear dup"));
    let two: Vec<String> = listing(&repos.remote(), &cleared)
        .into_iter()
        .filter(|row| row.starts_with("two.json"))
        .collect();
    assert_eq!(
        two,
        [
            format!("two.json {}", oid_of(&repos.bob, &task("two"))),
            format!("two.json {}", oid_of(&repos.bob, &other("two")))
        ],
        "the other name, every entry, exactly as it was"
    );
    let board = bob.read().unwrap();
    assert!(board.tasks.contains(&task("dup")));
    let refused = bob.write(&board.head, &[task("fresh")], "board: add fresh");
    assert!(
        matches!(refused, Ok(BoardWrite::Refused(_))),
        "two.json still refuses writes: {refused:?}"
    );
    let keep = oid_of(&repos.bob, &other("two"));
    let mended = written(bob.clear_set_aside(
        &board.head,
        "tasks/two.json",
        Some(&keep),
        "board: clear two",
    ));
    written(bob.write(&mended, &[task("fresh")], "board: add fresh"));
}

#[test]
fn a_file_set_aside_once_is_cleared_without_a_copy_to_keep() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    raw_board(
        &repos.remote(),
        &[
            ("100644", "broken.json", Object::Blob(b"{".to_vec())),
            (
                "100644",
                "ok.json",
                Object::Blob(encode(&task("ok")).unwrap()),
            ),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let head = bob.read().unwrap().head;
    let keep = oid_of(&repos.bob, &task("ok"));
    assert!(matches!(
        bob.clear_set_aside(
            &head,
            "tasks/broken.json",
            Some(&keep),
            "board: clear broken"
        ),
        Ok(BoardWrite::Refused(_))
    ));
    let cleared =
        written(bob.clear_set_aside(&head, "tasks/broken.json", None, "board: clear broken"));
    assert_eq!(
        listing(&repos.remote(), &cleared).len(),
        2,
        "ok.json and its summary"
    );
    written(bob.write(&cleared, &[task("broken")], "board: write broken"));
}
