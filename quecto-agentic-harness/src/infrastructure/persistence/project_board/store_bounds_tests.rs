//! The bounds a read takes, kept by every write: task-file bytes and tree
//! bytes.
use super::*;
use crate::infrastructure::persistence::project_board::task_record::MAX_TASK_FILE_BYTES;

/// A write that would hold more task-file bytes than a read takes is
/// refused before anything is built, so no valid write leaves a board no
/// one can read; a write up to the bound is written and read back.
#[test]
fn a_write_past_the_read_bound_is_invalid_and_one_at_it_is_written() {
    let repos = Repos::new();
    let size = |id: &str| encode(&task(id)).unwrap().len();
    let mut alice = store(&repos.alice);
    alice.limits.board_bytes = size("a") + size("b");
    let head = created(&alice);
    let full = written(alice.write(&head, &[task("a"), task("b")], "board: add a, b"));
    let mut bob = store(&repos.bob);
    bob.limits = alice.limits;
    assert_eq!(ids(&bob.read().unwrap()), ["a", "b"]);
    let refused = alice.write(&full, &[task("c")], "board: add c");
    assert!(
        matches!(&refused, Err(BoardStoreError::InvalidWrite(why)) if why.contains("bytes")),
        "{refused:?}"
    );
    let mut longer = task("a").into_fields();
    longer.title.push('!');
    let longer = Task::new(longer).unwrap();
    let refused = alice.write(&full, &[longer], "board: rename a");
    assert!(
        matches!(&refused, Err(BoardStoreError::InvalidWrite(_))),
        "{refused:?}"
    );
    written(alice.write(&full, &[task("a")], "board: write a again"));
}

/// Only task files a read reads count toward the read bound: files set
/// aside that a read skips (too big, not regular) never refuse a write,
/// and one a read still reads (undecodable) always counts.
#[test]
fn only_task_files_a_read_reads_count_toward_the_read_bound() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write(&head, &[task("a")], "board: add a"));
    let remote = repos.remote();
    let blob = |bytes: &[u8]| sh_with(&remote, &["hash-object", "-w", "--stdin"], bytes);
    let mut oversize = encode(&task("big")).unwrap();
    oversize.resize(MAX_TASK_FILE_BYTES + 1, b' ');
    let edited = hand_edit(
        &remote,
        &[
            ("big.json", format!("100644 blob {}", blob(&oversize))),
            (
                "exec.json",
                format!("100755 blob {}", blob(&encode(&task("exec")).unwrap())),
            ),
            ("broken.json", format!("100644 blob {}", blob(b"{"))),
        ],
    );
    let size = |id: &str| encode(&task(id)).unwrap().len();
    let at_bound = size("a") + size("b") + 1;
    for (bound, fits) in [(at_bound, true), (at_bound - 1, false)] {
        let mut bob = store(&repos.bob);
        bob.limits.board_bytes = bound;
        let write = bob.write(&edited, &[task("b")], "board: add b");
        match fits {
            true => assert!(
                matches!(write, Ok(BoardWrite::Written(_))),
                "{bound}: {write:?}"
            ),
            false => assert!(
                matches!(write, Err(BoardStoreError::InvalidWrite(_))),
                "{bound}: {write:?}"
            ),
        }
    }
}

/// A write that would make a tree larger than a read takes is refused, so
/// no write leaves a board no one can read.
#[test]
fn a_write_past_the_tree_bound_is_invalid_and_one_at_it_is_written() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    let one = written(alice.write(&head, &[task("a")], "board: add a"));
    // The larger of the board's two trees, its root and `tasks/`.
    let size = [format!("{}^{{tree}}", one.0), format!("{}:tasks", one.0)]
        .iter()
        .map(|tree| {
            sh(&repos.remote(), &["cat-file", "-s", tree])
                .parse::<usize>()
                .unwrap()
        })
        .max()
        .unwrap();
    let mut bob = store(&repos.bob);
    bob.limits.tree_bytes = size;
    let again = written(bob.write(&one, &[task("a")], "board: write a again"));
    let refused = bob.write(&again, &[task("b")], "board: add b");
    assert!(
        matches!(&refused, Err(BoardStoreError::InvalidWrite(why)) if why.contains("tree")),
        "{refused:?}"
    );
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), again.0);
}
