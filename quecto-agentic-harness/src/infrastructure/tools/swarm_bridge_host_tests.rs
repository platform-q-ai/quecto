//! The host's read of a board (#2278): the Rust board's alone, with no
//! Python and no process started.
use super::{context, create};
use serde_json::json;

/// The page faults of this process's waited-for children, from
/// `/proc/self/stat` (`cminflt` and `cmajflt`): every child that ran and
/// was reaped adds to them (its exec alone faults its pages in), and
/// nothing else does.
fn reaped_children_faults() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    // The fields after the command name, which ends at the last `)`: the
    // state is field 3, cminflt field 11 and cmajflt field 13.
    let fields: Vec<&str> = stat[stat.rfind(')').unwrap() + 1..]
        .split_whitespace()
        .collect();
    fields[8].parse::<u64>().unwrap() + fields[10].parse::<u64>().unwrap()
}

/// This process's children not yet reaped, running or exited, from every
/// thread's `/proc/self/task/<tid>/children`.
fn unreaped_children() -> String {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .map(|task| std::fs::read_to_string(task.unwrap().path().join("children")).unwrap())
        .collect()
}

/// The host reads a board with no Python (#2278): in a child process
/// whose `PATH` is an empty directory, where no interpreter can be found
/// (review M3), the read still succeeds, and it starts no process at all
/// (final review nit): no child of the reading process ran and was
/// reaped, and none is left running or unreaped.
#[test]
fn hosted_run_reads_without_python() {
    const CHILD: &str = "QUECTO_TEST_2278_NO_PYTHON_CHILD";
    if let Ok(checkout) = std::env::var(CHILD) {
        // No program can be found by name, an interpreter included: every
        // `PATH` entry is empty, so the read is the Rust board's alone.
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
        assert!(!dirs.is_empty(), "the child has a PATH");
        for dir in &dirs {
            assert!(
                std::fs::read_dir(dir).unwrap().next().is_none(),
                "{} holds no program",
                dir.display()
            );
        }
        // The measure sees a child that ran: this test binary, listing
        // no test, by its absolute path.
        let before = reaped_children_faults();
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--list", "--exact", "no-such-test"])
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        assert!(
            reaped_children_faults() > before,
            "a reaped child adds its page faults"
        );
        let before = reaped_children_faults();
        let hosted = super::super::swarm_bridge::HostedStore::at(
            checkout.into(),
            crate::composition::swarm::swarm_board(),
        );
        let run = hosted.hosted_run().unwrap().expect("a created run");
        assert_eq!(run.coordinator, "parent");
        assert_eq!(
            reaped_children_faults(),
            before,
            "the read ran no child process"
        );
        assert_eq!(
            unreaped_children().trim(),
            "",
            "the read left no child process"
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    create(&context(&directory), 1);
    let empty = tempfile::tempdir().unwrap();
    let name = std::thread::current().name().unwrap().to_owned();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture", "--test-threads=1"])
        .env(CHILD, directory.path())
        .env("PATH", empty.path())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The supervising session reads the whole board (#2478): the run, the
/// items with their reserved paths, the members, the totals, at the
/// board's revision, with no claim or reservation token, through the
/// board's own adapter.
#[test]
fn the_host_reads_the_board_projection_without_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let hosted = super::super::swarm_bridge::HostedStore::at(
        directory.path().into(),
        crate::composition::swarm::swarm_board(),
    );
    assert_eq!(
        hosted.board_projection().unwrap(),
        None,
        "no store, no board"
    );
    assert_eq!(hosted.board_revision().unwrap(), None);
    create(&context, 2);
    let tokens = super::super::call_work::off_the_runtime(|| {
        let create = |request: &str, title: &str, depends: serde_json::Value| {
            context
                .call("task_create", json!([request, title, ["passes"], depends]))
                .unwrap()
        };
        create("r1", "Plumbing", json!([]));
        create("r2", "CAS", json!([1]));
        let claimed = context.call("claim", json!([1])).unwrap();
        let reserved = context
            .call("reserve", json!([1, claimed["token"], ["src/a.rs"]]))
            .unwrap();
        [claimed["token"].clone(), reserved["token"].clone()]
    });
    let board = hosted.board_projection().unwrap().expect("a board");
    assert_eq!(board["revision"].as_i64(), hosted.board_revision().unwrap());
    assert_eq!(
        (board["run"]["goal"].clone(), board["run"]["status"].clone()),
        (json!("ship"), json!("running"))
    );
    assert_eq!(board["items"][0]["state"], json!("claimed"));
    assert_eq!(board["items"][0]["owner"], json!("parent"));
    assert_eq!(board["items"][0]["reserved_paths"], json!(["src/a.rs"]));
    assert_eq!(
        (
            board["items"][1]["state"].clone(),
            board["items"][1]["depends_on"].clone()
        ),
        (json!("blocked"), json!([1]))
    );
    assert_eq!(board["members"][0]["role"], json!("coordinator"));
    assert_eq!(board["members"][0]["claims"], json!([1]));
    assert_eq!(
        (
            board["totals"]["claimed"].clone(),
            board["totals"]["blocked"].clone()
        ),
        (json!(1), json!(1))
    );
    let text = board.to_string();
    for token in tokens {
        assert!(
            !text.contains(token.as_str().unwrap()),
            "a token crossed: {text}"
        );
    }
    let before = hosted.board_revision().unwrap().unwrap();
    super::super::call_work::off_the_runtime(|| {
        context
            .call("task_create", json!(["r3", "Index", ["ok"], []]))
            .unwrap()
    });
    assert!(
        hosted.board_revision().unwrap().unwrap() > before,
        "a write moves the revision"
    );
}
