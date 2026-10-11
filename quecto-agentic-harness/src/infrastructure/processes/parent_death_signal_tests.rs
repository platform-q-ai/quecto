use super::*;

#[tokio::test]
async fn armed_child_still_runs_when_the_parent_lives() {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg("exit 7");
    cmd.stdin(std::process::Stdio::null());
    arm(&mut cmd, std::process::id());
    let status = cmd.status().await.unwrap();
    assert_eq!(status.code(), Some(7));
}

#[tokio::test]
async fn a_wrong_expected_parent_makes_the_child_exit_before_exec() {
    // Simulates the race: the parent that forked is not the expected one.
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg("exit 0");
    cmd.stdin(std::process::Stdio::null());
    // pid 1 is never the test process.
    arm(&mut cmd, 1);
    let status = cmd.status().await.unwrap();
    #[cfg(target_os = "linux")]
    assert_eq!(status.code(), Some(1));
    #[cfg(not(target_os = "linux"))]
    assert_eq!(status.code(), Some(0));
}

#[tokio::test]
async fn arming_is_inert_for_a_zero_parent() {
    let mut cmd = tokio::process::Command::new("true");
    arm(&mut cmd, 0);
    assert!(cmd.status().await.unwrap().success());
}

/// One process a test watches: its pid and its start time, so a pid the
/// kernel reuses is never taken for it.
struct Watched {
    pid: String,
    start: String,
}

/// The start time and state of process `pid`, from `/proc/<pid>/stat`.
fn stat(pid: &str) -> Option<(String, char)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(") ")?;
    let fields: Vec<&str> = rest.split(' ').collect();
    Some((fields.get(19)?.to_string(), fields.first()?.chars().next()?))
}

impl Watched {
    /// No such process, another one of its pid, or a zombie no one reaped.
    fn gone(&self) -> bool {
        match stat(&self.pid) {
            Some((start, state)) => start != self.start || state == 'Z',
            None => true,
        }
    }
}

/// This test binary run as a quecto that a test then kills outright: the
/// child test it runs starts processes that write their pids to files.
/// Dropped, it kills the parent and every process it watches that still
/// runs, so a failing test leaves nothing behind.
pub(crate) struct KilledParent {
    parent: Option<std::process::Child>,
    watched: Vec<Watched>,
}

impl KilledParent {
    /// Runs test `child_test` of this binary alone with `vars`, and waits
    /// (at most twenty seconds) for each of `pid_files` to name a process.
    pub(crate) fn start(
        child_test: &str,
        vars: &[(&str, &std::ffi::OsStr)],
        pid_files: &[&std::path::Path],
    ) -> Self {
        let parent = std::process::Command::new(std::env::current_exe().unwrap())
            .args([child_test, "--exact", "--test-threads=1"])
            .envs(vars.iter().copied())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut started = Self {
            parent: Some(parent),
            watched: vec![],
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        for file in pid_files {
            let watched = loop {
                let pid = std::fs::read_to_string(file).map(|pid| pid.trim().to_string());
                match pid
                    .ok()
                    .and_then(|pid| stat(&pid).map(|(start, _)| (pid, start)))
                {
                    Some((pid, start)) => break Watched { pid, start },
                    None if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    None => panic!("{} never named a process", file.display()),
                }
            };
            started.watched.push(watched);
        }
        started
    }

    /// Kills the parent outright, then answers, for each process watched,
    /// whether it was gone within `bound`.
    pub(crate) fn kill(&mut self, bound: std::time::Duration) -> Vec<bool> {
        if let Some(mut parent) = self.parent.take() {
            parent.kill().unwrap();
            parent.wait().unwrap();
        }
        let deadline = std::time::Instant::now() + bound;
        while !self.watched.iter().all(Watched::gone) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.watched.iter().map(Watched::gone).collect()
    }
}

impl Drop for KilledParent {
    fn drop(&mut self) {
        if let Some(parent) = self.parent.as_mut() {
            let _ = parent.kill();
            let _ = parent.wait();
        }
        for watched in self.watched.iter().filter(|watched| !watched.gone()) {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", &watched.pid])
                .status();
        }
    }
}
