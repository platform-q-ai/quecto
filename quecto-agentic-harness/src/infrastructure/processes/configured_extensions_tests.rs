use super::*;
use crate::domain::agents::configured_extensions::AgentRole;

fn spec(name: &str, command: &str, args: &[&str]) -> ExtensionSpec {
    ExtensionSpec {
        name: name.into(),
        command: command.into(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        env: [("AGENT".to_string(), "{agent_id}".to_string())].into(),
        children: true,
    }
}

#[test]
fn a_plan_expands_each_instance_under_its_own_state_directory() {
    let extensions = AgentExtensions::select(
        vec![spec("bt", "/opt/bt", &["{socket}", "{state_dir}/profile"])],
        AgentRole::LocalChild,
        Some("c-1"),
        true,
    );
    let launches = plan(&extensions, Path::new("/run/c-1.sock"), Path::new("/base"));
    let launch = &launches[0];
    assert_eq!(launch.state_dir, Path::new("/base/extensions/bt/c-1"));
    assert_eq!(
        launch.log,
        Path::new("/base/extensions/bt/c-1/extension.log")
    );
    let spec = launch.spec.as_ref().unwrap();
    assert_eq!(
        spec.args,
        ["/run/c-1.sock", "/base/extensions/bt/c-1/profile"]
    );
    assert_eq!(spec.env["AGENT"], "c-1");
}

/// A launch that fails, and an exit with code 2, stop the extension for
/// good with a warning naming why; both settle the start-up wait.
#[tokio::test]
async fn a_stopped_extension_settles_and_warns_without_restarting() {
    let base = tempfile::tempdir().unwrap();
    let extensions = AgentExtensions::select(
        vec![
            spec("missing", "/nonexistent/extension", &[]),
            spec("usage", "/bin/sh", &["-c", "echo usage >&2; exit 2"]),
        ],
        AgentRole::TopLevel,
        None,
        true,
    );
    let launches = plan(&extensions, Path::new("/run/a.sock"), base.path());
    let host = ConfiguredExtensions::launch(launches, OwnedChildSupervisor::process_wide());
    tokio::time::timeout(Duration::from_secs(20), host.settled())
        .await
        .expect("settled");
    let warnings = host.warnings().join("\n");
    assert!(
        warnings.contains("`missing` could not be launched"),
        "{warnings}"
    );
    assert!(
        warnings.contains("`usage` exited with code 2"),
        "{warnings}"
    );
    let log = base.path().join("extensions/usage/main/extension.log");
    assert_eq!(std::fs::read_to_string(log).unwrap(), "usage\n");
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let state_dir = base.path().join("extensions/usage/main");
    assert_eq!(mode(&state_dir), 0o700, "an instance's state is private");
    assert_eq!(mode(&state_dir.join("extension.log")), 0o600);
    assert!(host.revision() > 0);
    assert!(
        host.claim(Some(i32::try_from(std::process::id()).unwrap()))
            .is_none()
    );
    host.shutdown().await;
}

/// #2446 review L4: shutdown waits for every supervisor, one mid-launch
/// included, so no instance outlives it; a sub-agent's state goes.
#[tokio::test]
async fn shutdown_ends_every_instance_and_a_childs_state() {
    for settle_first in [false, true] {
        let base = tempfile::tempdir().unwrap();
        let pid_file = base.path().join("pid");
        let script = format!("echo $$ > {}; exec sleep 60", pid_file.display());
        let extensions = AgentExtensions::select(
            vec![spec("sleeper", "/bin/sh", &["-c", &script])],
            AgentRole::LocalChild,
            Some("c-1"),
            true,
        );
        let launches = plan(&extensions, Path::new("/run/a.sock"), base.path());
        let host = ConfiguredExtensions::launch(launches, OwnedChildSupervisor::process_wide());
        if settle_first {
            for _ in 0..200 {
                if std::fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n')) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
        host.shutdown().await;
        if let Ok(pid) = std::fs::read_to_string(&pid_file) {
            let pid: i32 = pid.trim().parse().unwrap();
            // SAFETY: signal 0 only probes existence.
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            assert!(!alive, "the instance ended with shutdown");
        }
        assert!(!base.path().join("extensions/sleeper/c-1").exists());
    }
}
