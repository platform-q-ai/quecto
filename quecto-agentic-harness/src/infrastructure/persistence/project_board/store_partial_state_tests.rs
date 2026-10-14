//! What a partial clone remembers between reads (#2481 S10b review round
//! 2): a user's own fetch first, a new file past every cap, and a state
//! file that is absent, damaged, foreign to the clone or past its bounds.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::FILTER_BYTES;

/// The remote's `git-upload-pack` as `clone` runs it, counted, with every
/// request `clone` sends it kept.
struct Watch {
    count: PathBuf,
    log: PathBuf,
}

impl Watch {
    fn on(repos: &Repos, clone: &Path) -> Self {
        let watch = Self {
            count: repos.dir.path().join("upload-packs"),
            log: repos.dir.path().join("requests"),
        };
        let wrapper = repos.dir.path().join("upload-pack");
        executable(
            &wrapper,
            &format!(
                "#!/bin/sh\necho x >> '{}'\ntee -a '{}' | git-upload-pack \"$@\"\n",
                watch.count.display(),
                watch.log.display()
            ),
        );
        let wrapper = wrapper.to_str().unwrap();
        sh(clone, &["config", "remote.origin.uploadpack", wrapper]);
        watch
    }

    fn runs(&self) -> usize {
        std::fs::read_to_string(&self.count).map_or(0, |text| text.lines().count())
    }

    /// Whether `clone` asked for `oid` since the watch began.
    fn wanted(&self, oid: &str) -> bool {
        let log = std::fs::read(&self.log).unwrap_or_default();
        let want = format!("want {oid}");
        log.windows(want.len())
            .any(|window| window == want.as_bytes())
    }
}

/// Where `clone` keeps its state for `origin`.
fn state_file(clone: &Path) -> PathBuf {
    clone.join(".git/quecto/project-board/fetch-origin")
}

fn put_state(clone: &Path, text: &str) {
    let path = state_file(clone);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Plan 1 of task `a`, a file `a.2.md` past every cap and `files` in
/// `plans/`, and run 1 of task `a` in `runs/`.
fn board_with(repos: &Repos, files: Vec<(&'static str, String, Vec<u8>)>) -> BoardHead {
    board_of(repos, |_| {
        let mut plans = vec![
            regular("a.1.md", bytes::<PlanCodec>(&plan("a", 1))),
            regular("a.2.md", vec![b'x'; 2 * 1024 * 1024]),
        ];
        plans.extend(files.clone());
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", Node::Tree(plans)),
            (
                "40000",
                "runs",
                Node::Tree(vec![regular("a.1.md", bytes::<RunCodec>(&run("a", 1)))]),
            ),
        ]
    })
}

/// When the user's own fetch moves the board first, quecto's fetch brings
/// nothing (git skips a head it holds): only the folders that changed are
/// fetched again, never one that did not, and never the file past the cap.
#[test]
fn a_user_fetch_first_refetches_only_the_folders_that_changed() {
    let repos = fresh();
    let head = board_with(&repos, vec![]);
    let carol = partial(&repos, "tree:0", "carol");
    store(&carol).read().unwrap();
    let alice = store(&repos.alice);
    let three = [BoardArtifact::Plan(plan("a", 3))];
    let moved = written(alice.write(&head, &three, "board: plan a 3"));
    sh(&carol, &["fetch", "-q", "origin"]);
    let watch = Watch::on(&repos, &carol);
    let board = store(&carol).read().unwrap();
    assert_eq!(board.plans, [plan("a", 1), plan("a", 3)]);
    let remote = repos.remote();
    assert!(
        watch.wanted(&at(&remote, &moved, "plans")),
        "plans/ changed"
    );
    assert!(!watch.wanted(&at(&remote, &moved, "runs")), "runs/ did not");
    assert!(!held(&carol, &at(&remote, &head, "plans/a.2.md")));
}

/// A new file past every cap costs one more round once: the next head
/// knows it, and is one fetch again.
#[test]
fn a_new_file_past_the_cap_is_fetched_around_once() {
    let repos = fresh();
    board_with(&repos, vec![]);
    let carol = partial(&repos, "tree:0", "carol");
    store(&carol).read().unwrap();
    let big = vec![b'y'; 2 * 1024 * 1024];
    board_with(&repos, vec![regular("a.5.md", big.clone())]);
    assert_eq!(store(&carol).read().unwrap().plans, [plan("a", 1)]);
    let three = bytes::<PlanCodec>(&plan("a", 3));
    board_with(
        &repos,
        vec![regular("a.5.md", big.clone()), regular("a.3.md", three)],
    );
    let watch = Watch::on(&repos, &carol);
    let board = store(&carol).read().unwrap();
    assert_eq!(board.plans, [plan("a", 1), plan("a", 3)]);
    assert!(reason(&board, "plans/a.5.md").contains("at least"));
    assert_eq!(watch.runs(), 2, "the head, and one fetch of what changed");
}

/// A first read of a clone made with no trees takes four runs: the head,
/// the commits, the root, then the folders with the root's files.
#[test]
fn a_first_read_without_trees_is_four_runs() {
    let repos = fresh();
    board_with(&repos, vec![]);
    let carol = partial(&repos, "tree:0", "carol");
    let watch = Watch::on(&repos, &carol);
    store(&carol).read().unwrap();
    assert_eq!(watch.runs(), 4);
}

/// A state the clone cannot use is no state: one naming a commit the
/// clone lacks, one of another filter's limit, and garbage. Each read is
/// whole, and leaves a state of its own.
#[test]
fn a_state_the_clone_cannot_use_is_no_state() {
    let absent = "0123456789abcdef0123456789abcdef01234567";
    for (name, text) in [
        (
            "absent",
            format!("quecto-board-fetch 1 {FILTER_BYTES}\nread {absent}\n"),
        ),
        (
            "limit",
            format!(
                "quecto-board-fetch 1 {}\nomitted {absent}\n",
                FILTER_BYTES - 1
            ),
        ),
        ("garbage", "garbage\n".to_string()),
    ] {
        let repos = fresh();
        let head = board_with(&repos, vec![]);
        let carol = partial(&repos, "tree:0", "carol");
        put_state(&carol, &text);
        let board = store(&carol).read().unwrap();
        assert_eq!(board.plans, [plan("a", 1)], "{name}");
        let kept = std::fs::read_to_string(state_file(&carol)).unwrap();
        assert!(
            kept.starts_with(&format!(
                "quecto-board-fetch 1 {FILTER_BYTES}\nread {}\n",
                head.0
            )),
            "{name}: {kept}"
        );
    }
}

/// A full clone never reads a state: one naming its lost file as left on
/// the remote still has it fetched whole.
#[test]
fn a_full_clone_takes_no_state() {
    let repos = fresh();
    let head = board_with(&repos, vec![]);
    serve_filters(&repos.remote());
    let oid = at(&repos.remote(), &head, "plans/a.2.md");
    let loose = repos
        .bob
        .join(".git/objects")
        .join(&oid[..2])
        .join(&oid[2..]);
    std::fs::remove_file(loose).unwrap();
    put_state(
        &repos.bob,
        &format!(
            "quecto-board-fetch 1 {FILTER_BYTES}\nread {}\nomitted {oid}\n",
            head.0
        ),
    );
    let board = store(&repos.bob).read().unwrap();
    assert_eq!(
        reason(&board, "plans/a.2.md"),
        "plan file is 2097152 bytes; the most is 1572864"
    );
    assert!(held(&repos.bob, &oid));
}

/// A state is kept only when it changes, and a state that cannot be kept
/// leaves no draft behind.
#[test]
fn a_state_is_kept_when_it_changes_and_leaves_no_draft() {
    use std::os::unix::fs::MetadataExt;
    let repos = fresh();
    board_with(&repos, vec![]);
    let carol = partial(&repos, "tree:0", "carol");
    store(&carol).read().unwrap();
    let inode = || std::fs::metadata(state_file(&carol)).unwrap().ino();
    let first = inode();
    store(&carol).read().unwrap();
    assert_eq!(inode(), first, "the same head keeps the same state");
    board_with(&repos, vec![regular("a.3.md", b"x".to_vec())]);
    let path = state_file(&carol);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    store(&carol).read().unwrap();
    let left: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["fetch-origin"], "no draft is left behind");
}

/// A full clone keeps no state at all.
#[test]
fn a_full_clone_keeps_no_state() {
    let repos = fresh();
    board_with(&repos, vec![]);
    store(&repos.bob).read().unwrap();
    assert!(!state_file(&repos.bob).exists());
}

/// A read of an older head never overwrites a state another read kept of
/// a newer one, a descendant of it.
#[test]
fn a_state_of_a_newer_head_is_never_overwritten_by_an_older_one() {
    let repos = fresh();
    let older = board_with(&repos, vec![]);
    let carol = partial(&repos, "tree:0", "carol");
    store(&carol).read().unwrap();
    let alice = store(&repos.alice);
    let three = [BoardArtifact::Plan(plan("a", 3))];
    let newer = written(alice.write(&older, &three, "board: plan a 3"));
    store(&carol).read().unwrap();
    let kept = std::fs::read_to_string(state_file(&carol)).unwrap();
    assert!(kept.contains(&format!("read {}\n", newer.0)), "{kept}");
    sh(&repos.remote(), &["update-ref", BOARD_REF, &older.0]);
    assert_eq!(store(&carol).read().unwrap().head, older);
    assert_eq!(std::fs::read_to_string(state_file(&carol)).unwrap(), kept);
}

/// A keeping sweeps the drafts dead keepings left of this remote's state
/// (older than any keeping lasts), and only those: a fresh draft, and any
/// other file however old, stay.
#[test]
fn a_keeping_sweeps_only_drafts_dead_keepings_left() {
    let repos = fresh();
    board_with(&repos, vec![]);
    let carol = partial(&repos, "tree:0", "carol");
    let dir = state_file(&carol).parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    let names = [
        "fetch-origin.4242.7.draft",
        "fetch-origin.4243.8.draft",
        "fetch-other.4242.7.draft",
        "fetch-origin.4242.x.draft",
        "notes.txt",
    ]
    .map(String::from);
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for (n, name) in names.iter().enumerate() {
        let file = std::fs::File::create(dir.join(name)).unwrap();
        // All but the second are old: it is a keeping still under way.
        if n != 1 {
            file.set_modified(hour_ago).unwrap();
        }
    }
    store(&carol).read().unwrap();
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    let mut expected: Vec<String> = names[1..].to_vec();
    expected.push("fetch-origin".into());
    expected.sort();
    assert_eq!(left, expected);
}
