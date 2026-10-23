//! Contract for the `AuthorityLauncher` port (#2615): the probe answers
//! only while a broker accepts at the directory; a launch starts a broker
//! that comes to answer it; a program other than quecto is never launched.
//! The adapter under test launches the real `quecto` binary, tethered to
//! the test thread so no broker outlives the test (#2631). How a launched
//! broker reads is `launched_authority`'s contract.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use quecto::application::admission::ports::{AuthorityLaunch, AuthorityLauncher};
use quecto::infrastructure::admission::DetachedAuthorityLauncher;

const LIMIT: Duration = Duration::from_secs(20);

pub(crate) fn launch_for(dir: &Path) -> AuthorityLaunch {
    let authority = dir.join("authority");
    std::fs::write(
        dir.join("config.json"),
        format!(
            r#"{{"providers":{{"openai_compatible":{{"endpoints":[{{"prefix":"fake","api_key":"fake-key","api_base":"http://127.0.0.1:1","allow_remote_http":true}}]}}}},"admission":{{"directory":{authority:?},"groups":{{"g":{{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":8,"queue_timeout_ms":1000,"attempt_timeout_ms":2000,"fallback_base_ms":50,"max_cooldown_ms":500}}}},"aliases":{{"acct":"g"}},"bindings":{{"fake":"acct"}}}}}}"#,
            authority = authority.to_string_lossy()
        ),
    )
    .unwrap();
    AuthorityLaunch {
        directory: authority,
        config: dir.join("config.json"),
        base_dir: dir.to_path_buf(),
    }
}

#[test]
fn a_launched_broker_comes_to_answer_the_probe() {
    let temp = tempfile::tempdir().unwrap();
    let launch = launch_for(temp.path());
    let launcher = DetachedAuthorityLauncher::dying_with_launching_thread(PathBuf::from(env!(
        "CARGO_BIN_EXE_quecto"
    )));
    assert!(
        launcher.probe(&launch.directory).is_err(),
        "nothing serves yet"
    );
    let _broker = launcher.launch(&launch).unwrap();
    let deadline = Instant::now() + LIMIT;
    let mut attempt = 0;
    while launcher.probe(&launch.directory).is_err() {
        assert!(
            Instant::now() < deadline,
            "the launched broker never answered"
        );
        launcher.pause(attempt.min(3));
        attempt += 1;
    }
    let log =
        std::fs::read_to_string(DetachedAuthorityLauncher::log_path(&launch.directory)).unwrap();
    assert!(log.contains("admission authority ready"), "{log}");
}

#[test]
fn only_the_quecto_binary_is_launched() {
    let temp = tempfile::tempdir().unwrap();
    let launch = launch_for(temp.path());
    let launcher =
        DetachedAuthorityLauncher::dying_with_launching_thread(PathBuf::from("/bin/true"));
    let error = launcher.launch(&launch).err().expect("refused");
    assert!(error.contains("is not the quecto binary"), "{error}");
}
