//! The host's read of a board (#2278): the Rust board's alone, with no
//! Python and no process started.
use super::{context, create};

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
/// whose `PATH` is an empty directory, where no `python3` can be found
/// (review M3), the read still succeeds, and it starts no process at all
/// (final review nit): no child of the reading process ran and was
/// reaped, and none is left running or unreaped.
#[test]
fn hosted_run_reads_without_python() {
    const CHILD: &str = "QUECTO_TEST_2278_NO_PYTHON_CHILD";
    if let Ok(checkout) = std::env::var(CHILD) {
        // No interpreter can be found: the read is the Rust board's alone.
        assert!(
            std::process::Command::new("python3")
                .arg("-c")
                .arg("pass")
                .status()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "python3 is not on this child's PATH"
        );
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
