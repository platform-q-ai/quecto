//! The admission broker starts on demand (#2615, ADR-0032), against the
//! real `quecto` binary: an unreachable authority is started detached (its
//! own session and process group, output in the admission directory's log)
//! and joined; concurrent starters end up with one broker and both join it.
//!
//! Every broker here is launched tethered to the test thread that launched
//! it, so none outlives its test (#2631); a guard also stops the ones this
//! test's directory names when it ends.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use quecto::application::admission::dto::negotiation::AuthorityAvailability;
use quecto::application::admission::ports::AuthorityLaunch;
use quecto::application::admission::use_cases::NegotiateAuthority;
use quecto::infrastructure::admission::{DetachedAuthorityLauncher, Negotiation, negotiate};

const LIMIT: Duration = Duration::from_secs(20);

fn quecto_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_quecto"))
}

/// A global config whose admission section serves `<dir>/authority`.
fn write_config(dir: &Path) -> AuthorityLaunch {
    let authority = dir.join("authority");
    let config = format!(
        r#"{{"providers":{{"openai_compatible":{{"endpoints":[{{"prefix":"fake","api_key":"fake-key","api_base":"http://127.0.0.1:1","allow_remote_http":true}}]}}}},"admission":{{"directory":{authority:?},"groups":{{"g":{{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":8,"queue_timeout_ms":1000,"attempt_timeout_ms":2000,"fallback_base_ms":50,"max_cooldown_ms":500}}}},"aliases":{{"acct":"g"}},"bindings":{{"fake":"acct"}}}}}}"#,
        authority = authority.to_string_lossy()
    );
    std::fs::write(dir.join("config.json"), config).unwrap();
    AuthorityLaunch {
        directory: authority,
        config: dir.join("config.json"),
        base_dir: dir.to_path_buf(),
    }
}

fn use_case() -> NegotiateAuthority {
    NegotiateAuthority::new(Arc::new(
        DetachedAuthorityLauncher::dying_with_launching_thread(quecto_binary()),
    ))
}

/// The pids of the brokers serving `directory`, read from `/proc`.
fn brokers_for(directory: &Path) -> Vec<i32> {
    let needle = directory.to_string_lossy().to_string();
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|cmdline| {
                let args: Vec<String> = cmdline
                    .split(|byte| *byte == 0)
                    .map(|arg| String::from_utf8_lossy(arg).to_string())
                    .collect();
                args.iter().any(|arg| arg == "admission-broker") && args.contains(&needle)
            })
        })
        .collect()
}

/// Wait (bounded) until exactly one broker serves `directory`.
fn one_broker(directory: &Path) -> i32 {
    let deadline = Instant::now() + LIMIT;
    loop {
        match brokers_for(directory).as_slice() {
            [pid] => return *pid,
            others => {
                assert!(Instant::now() < deadline, "brokers: {others:?}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Stops the test's own brokers (only those its temporary directory names)
/// when the test ends, however it ends.
struct StopTestBrokers(PathBuf);

impl Drop for StopTestBrokers {
    fn drop(&mut self) {
        for pid in brokers_for(&self.0) {
            // SAFETY: kill(2) on a pid this test's own directory names.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

fn stat_fields(pid: i32) -> Vec<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    // Fields after the parenthesised command name: state, ppid, pgrp, session.
    let after = stat.rsplit_once(") ").unwrap().1;
    after.split(' ').map(str::to_string).collect()
}

fn log(launch: &AuthorityLaunch) -> String {
    std::fs::read_to_string(DetachedAuthorityLauncher::log_path(&launch.directory))
        .unwrap_or_default()
}

#[test]
fn an_unreachable_authority_is_started_detached_and_joined() {
    let temp = tempfile::tempdir().unwrap();
    let launch = write_config(temp.path());
    let _stop = StopTestBrokers(launch.directory.clone());
    let use_case = use_case();
    assert_eq!(
        use_case.ensure_running(&launch),
        Ok(AuthorityAvailability::StartedOnDemand),
        "{}",
        log(&launch)
    );
    let binding = negotiate(Negotiation::Root {
        directory: launch.directory.clone(),
    })
    .expect("the root joins the broker it started");
    assert!(binding.connected());
    let pid = one_broker(&launch.directory);
    let fields = stat_fields(pid);
    assert_eq!(fields[2], pid.to_string(), "it leads its process group");
    assert_eq!(fields[3], pid.to_string(), "it leads its session");
    let log = log(&launch);
    assert!(log.contains("admission authority ready"), "{log}");
    // Reachable now: nothing more is started.
    assert_eq!(
        use_case.ensure_running(&launch),
        Ok(AuthorityAvailability::AlreadyRunning)
    );
    assert_eq!(brokers_for(&launch.directory), vec![pid]);
}

#[test]
fn concurrent_starters_end_with_one_broker_and_both_join_it() {
    let temp = tempfile::tempdir().unwrap();
    let launch = write_config(temp.path());
    let _stop = StopTestBrokers(launch.directory.clone());
    let start = Barrier::new(2);
    let joined = Barrier::new(3);
    let done = Barrier::new(3);
    let started = std::thread::scope(|scope| {
        let starters: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    let availability = use_case().ensure_running(&launch);
                    let binding = negotiate(Negotiation::Root {
                        directory: launch.directory.clone(),
                    });
                    joined.wait();
                    // A broker dies with the thread that launched it: hold
                    // both threads until the test has looked.
                    done.wait();
                    (availability, binding.map(|binding| binding.connected()))
                })
            })
            .collect();
        joined.wait();
        one_broker(&launch.directory);
        done.wait();
        starters
            .into_iter()
            .map(|starter| starter.join().unwrap())
            .collect::<Vec<_>>()
    });
    let log = log(&launch);
    let mut launched = 0;
    for (availability, connected) in started {
        assert_eq!(connected, Ok(true), "both join the one broker: {log}");
        match availability {
            Ok(AuthorityAvailability::StartedOnDemand) => launched += 1,
            Ok(AuthorityAvailability::AlreadyRunning) => {}
            Err(error) => panic!("{error}\n{log}"),
        }
    }
    assert!(launched >= 1, "{log}");
    assert_eq!(
        log.matches("admission authority ready").count(),
        1,
        "one broker served: {log}"
    );
    assert!(
        log.matches("another admission authority owns").count() >= launched - 1,
        "every other start lost the lock: {log}"
    );
}
