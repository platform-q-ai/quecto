//! #2344 final review: the golden MANIFEST only shrinks, against the base
//! branch too (`scripts/check-golden-manifest.sh`, run by CI and the
//! pre-push hook): against the merge base's MANIFEST no line may change or
//! be added, and `GOLDEN_CEILING` may only go down. A fixture edited with
//! its MANIFEST line, or a new one with a raised ceiling, fails.
use std::path::Path;
use std::process::{Command, Output};

const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scripts/check-golden-manifest.sh"
);
const MANIFEST: &str = "quecto-agentic-harness/tests/fixtures/swarm_board/golden/MANIFEST";
const GOLDENS: &str = "quecto-agentic-harness/tests/integration/swarm_board_goldens.rs";

/// Runs `program` in `dir` with git isolated from the user's config and
/// from any `GIT_*` state a hook running this test left.
fn run(dir: &Path, program: &str, args: &[&str]) -> Output {
    let mut command = Command::new(program);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("the program runs")
}

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A repository whose base commit holds `manifest` and `ceiling`, then a
/// head commit holding the other two; the check's exit against the base.
fn checked(base: Option<(&str, usize)>, head: (&str, usize)) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    assert!(
        run(repo, "git", &["init", "-q", "-b", "master"])
            .status
            .success()
    );
    write(repo, "README", "base\n");
    if let Some((manifest, ceiling)) = base {
        write(repo, MANIFEST, manifest);
        write(
            repo,
            GOLDENS,
            &format!("const GOLDEN_CEILING: usize = {ceiling};\n"),
        );
    }
    assert!(run(repo, "git", &["add", "-A"]).status.success());
    assert!(
        run(repo, "git", &["commit", "-q", "-m", "base"])
            .status
            .success()
    );
    write(repo, MANIFEST, head.0);
    write(
        repo,
        GOLDENS,
        &format!("const GOLDEN_CEILING: usize = {};\n", head.1),
    );
    assert!(run(repo, "git", &["add", "-A"]).status.success());
    assert!(
        run(
            repo,
            "git",
            &["commit", "-q", "--allow-empty", "-m", "head"]
        )
        .status
        .success()
    );
    run(repo, "bash", &[SCRIPT, "master~1"])
}

const TWO: &str = "aa  s/1.json\nbb  s/2.json\n";

#[test]
fn the_golden_manifest_may_only_shrink_against_its_base() {
    for (head, passes, why) in [
        ((TWO, 2), true, "unchanged"),
        (
            ("aa  s/1.json\n", 1),
            true,
            "a line removed, the ceiling lowered",
        ),
        (("aa  s/1.json\n", 2), true, "a line removed"),
        (
            ("aa  s/1.json\ncc  s/2.json\n", 2),
            false,
            "a fixture's digest changed",
        ),
        (
            ("aa  s/1.json\nbb  s/2.json\ndd  s/3.json\n", 2),
            false,
            "a line added",
        ),
        ((TWO, 3), false, "the ceiling raised"),
    ] {
        let output = checked(Some((TWO, 2)), head);
        assert_eq!(
            output.status.success(),
            passes,
            "{why}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // The base without a MANIFEST: the change that introduces it.
    assert!(checked(None, (TWO, 2)).status.success());
}
