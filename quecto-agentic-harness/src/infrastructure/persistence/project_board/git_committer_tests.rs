use super::{ConfigAnswer, config_answer};
use crate::infrastructure::persistence::project_board::git::{
    Git, GitEnvironment, LOCAL_TIMEOUT, NETWORK_TIMEOUT,
};
use std::time::Duration;

/// #2666 round 4: a bound lowers both of git's timeouts to it, and never
/// raises one.
#[test]
fn a_bound_lowers_both_timeouts_and_never_raises_them() {
    let dir = tempfile::tempdir().unwrap();
    let vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("HOME".into(), dir.path().into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    let status = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .env_clear()
        .envs(vars.iter().cloned())
        .status()
        .unwrap();
    assert!(status.success());
    let git = Git::reader(dir.path(), GitEnvironment::allowlisted(vars)).unwrap();
    let tight = git.clone().bounded(Duration::from_secs(15));
    assert_eq!(
        (tight.local_timeout, tight.network_timeout),
        (Duration::from_secs(15), Duration::from_secs(15))
    );
    let loose = git.bounded(Duration::from_secs(3_600));
    assert_eq!(
        (loose.local_timeout, loose.network_timeout),
        (LOCAL_TIMEOUT, NETWORK_TIMEOUT)
    );
}

#[test]
fn only_exit_one_with_no_message_is_unset() {
    for (code, stdout, stderr, answer) in [
        (Some(0), "Ada\n", "", ConfigAnswer::Set("Ada".into())),
        (
            Some(0),
            "Ada\n",
            "warning: x",
            ConfigAnswer::Set("Ada".into()),
        ),
        (Some(1), "", "", ConfigAnswer::Unset),
        (
            Some(1),
            "",
            "error: bad config line 3",
            ConfigAnswer::Failed,
        ),
        (
            Some(128),
            "",
            "fatal: not a git repository",
            ConfigAnswer::Failed,
        ),
        (Some(2), "", "", ConfigAnswer::Failed),
        (None, "", "", ConfigAnswer::Failed),
    ] {
        assert_eq!(
            config_answer(code, stdout.as_bytes(), stderr.as_bytes()),
            answer,
            "{code:?} {stderr:?}"
        );
    }
}
