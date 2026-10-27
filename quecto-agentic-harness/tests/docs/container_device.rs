//! The official container adapter's `--device` flag: an allowlist of host
//! devices a container config may hand its container (`/dev/kvm` only, so a
//! swarm can run a headless Android emulator). The flag is refused for any
//! other spelling before any environment state exists; the preflight proves
//! the device is present and read-write for the invoking user; the run
//! forwards exactly the accepted devices; the create result reports them.
//!
//! The adapter runs against a recording fake runtime CLI, as in
//! `container_runtime_docs`. The production script always judges the real
//! `/dev/kvm`; so the tests do not depend on the host's, each runs a
//! temporary copy whose one probe-path line (asserted to occur exactly
//! once) looks under a temporary root instead. What is forwarded is always
//! the allowlisted path itself.
#![cfg(unix)]

use quecto::infrastructure::test_support::executable::write_executable;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const CREATE: &str = "scripts/container-runtime/docker/create.sh";
const REFUSAL: &str = "--device: only /dev/kvm is supported";
/// The one line of the production script that names where the device
/// check looks; the test copy rewrites it.
const PROBE_ANCHOR: &str = "    probe_path=\"$dev\"\n";

fn workspace_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above the harness crate")
        .join(relative)
}

/// A temporary host: a fake runtime CLI named `cli_name` on PATH that
/// records its `run` argv, a HOME, a socket dir, a state dir and a device
/// root the preflight looks under.
struct Host {
    dir: tempfile::TempDir,
    /// What the fake runtime's `info` prints, and its exit code.
    oci_runtime: &'static str,
    oci_exit: i32,
    /// The `%a %u %g %G` line the `stat` stand-in answers for the device;
    /// `None` leaves the real `stat`'s answer.
    stat_line: Option<String>,
}

impl Host {
    fn new(cli_name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        write_executable(
            &bin.join(cli_name),
            format!(
                "#!/usr/bin/env bash\nif [ \"$1\" = info ]; then printf '%s\\n' \"$FAKE_OCI_RUNTIME\"; exit \"$FAKE_OCI_EXIT\"; fi\nif [ \"$1\" = run ] && [ \"$2\" != --rm ]; then printf '%s\\n' \"$@\" > '{}'; fi\nexit 0\n",
                dir.path().join("cli-argv.txt").display()
            ),
        );
        // A `stat` stand-in: `-L -c` answers FAKE_STAT when it is set (the
        // device's mode, owner and group), anything else is the real one.
        write_executable(
            &bin.join("stat"),
            "#!/usr/bin/env bash\nif [ -n \"${FAKE_STAT:-}\" ] && [ \"$1\" = -L ] && [ \"$2\" = -c ]; then printf '%s\\n' \"$FAKE_STAT\"; exit 0; fi\nfor real in /usr/bin/stat /bin/stat; do [ -x \"$real\" ] && exec \"$real\" \"$@\"; done\nexit 127\n",
        );
        fs::create_dir_all(dir.path().join("home/.quecto")).unwrap();
        fs::create_dir_all(dir.path().join("sockets")).unwrap();
        fs::create_dir_all(dir.path().join("devices/dev")).unwrap();
        let production = fs::read_to_string(workspace_file(CREATE)).unwrap();
        assert_eq!(
            production.matches(PROBE_ANCHOR).count(),
            1,
            "create.sh names the probe path on exactly one line"
        );
        // No other line reads the device path for a probe: the anchor is
        // the only place the test copy moves.
        let reads: Vec<&str> = production
            .lines()
            .map(str::trim)
            .filter(|line| line.contains("\"$dev\""))
            .collect();
        assert_eq!(
            reads,
            [
                "is_allowed_device \"$dev\" || die \"device $dev escaped the allowlist\"",
                PROBE_ANCHOR.trim(),
                "run_as+=(--device \"$dev\")",
            ]
        );
        let copy = production.replace(
            PROBE_ANCHOR,
            &format!(
                "    probe_path=\"{}$dev\"\n",
                dir.path().join("devices").display()
            ),
        );
        write_executable(&dir.path().join("create.sh"), copy);
        Self {
            dir,
            oci_runtime: "crun",
            oci_exit: 0,
            stat_line: None,
        }
    }

    fn with_oci_runtime(mut self, runtime: &'static str) -> Self {
        self.oci_runtime = runtime;
        self
    }

    fn with_oci_exit(mut self, code: i32) -> Self {
        self.oci_exit = code;
        self
    }

    /// The device is a read-write character device (/dev/null) whose
    /// mode, owner and group `stat` reports as `line`.
    fn with_device_stat(self, line: &str) -> Self {
        self.device_is(Path::new("/dev/null"));
        Self {
            stat_line: Some(line.to_string()),
            ..self
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    /// Makes `<device root>/dev/kvm` the host device `target`.
    fn device_is(&self, target: &Path) {
        std::os::unix::fs::symlink(target, self.path("devices/dev/kvm")).unwrap();
    }

    fn state_entries(&self) -> Vec<String> {
        match fs::read_dir(self.path("state")) {
            Ok(entries) => entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("env-"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn run(&self, cli_name: &str, flags: &[&str], preflight_only: bool) -> Output {
        let mut command = std::process::Command::new(self.path("create.sh"));
        command
            .arg("--state-dir")
            .arg(self.path("state"))
            .args(flags);
        if preflight_only {
            command.arg("--preflight-only");
        } else {
            command
                .args(["--", "/bin/true", "--socket"])
                .arg(self.path("sockets/child.sock"));
        }
        let path = format!(
            "{}:{}",
            self.path("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        command
            .env("PATH", path)
            .env("HOME", self.path("home"))
            .env("QUECTO_CONTAINER_CLI", cli_name)
            .env_remove("QUECTO_REPO_CHECK_TIMEOUT")
            .env("FAKE_OCI_RUNTIME", self.oci_runtime)
            .env("FAKE_OCI_EXIT", self.oci_exit.to_string())
            .env("FAKE_STAT", self.stat_line.as_deref().unwrap_or(""))
            .env("QUECTO_CONTAINER_ENVIRONMENT_REF", "C1")
            .env_remove("QUECTO_ADMISSION_DIR")
            // The script reads git's global identity: never the caller's
            // repository or configuration.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("XDG_CONFIG_HOME", self.path("home/.config"))
            .output()
            .expect("run docker create.sh")
    }

    fn run_argv(&self) -> Vec<String> {
        fs::read_to_string(self.path("cli-argv.txt"))
            .expect("the fake CLI recorded a run invocation")
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The preflight's `(status, check, detail, remedy)` lines.
fn checks(output: &Output) -> Vec<Vec<String>> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains('\t'))
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

fn device_check(output: &Output) -> Vec<String> {
    checks(output)
        .into_iter()
        .find(|fields| fields[1] == "device")
        .unwrap_or_else(|| {
            panic!(
                "no device check: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
}

fn device_pairs(argv: &[String]) -> Vec<&str> {
    argv.windows(2)
        .filter(|pair| pair[0] == "--device")
        .map(|pair| pair[1].as_str())
        .collect()
}

#[test]
fn a_device_outside_the_allowlist_is_refused_before_any_environment_state() {
    for device in [
        "/dev/sda",
        "dev/kvm",
        "kvm",
        "/dev/kvm/",
        "/dev//kvm",
        "/dev/../dev/kvm",
        "/dev/kvm:/dev/kvm",
        "",
    ] {
        for preflight_only in [false, true] {
            let host = Host::new("fake-runtime");
            host.device_is(Path::new("/dev/null"));
            let output = host.run("fake-runtime", &["--device", device], preflight_only);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{device:?}: {}",
                stderr(&output)
            );
            assert!(
                stderr(&output).contains(REFUSAL),
                "{device:?}: {}",
                stderr(&output)
            );
            assert!(checks(&output).is_empty(), "refused before any check");
            assert!(host.state_entries().is_empty(), "{device:?}");
            assert!(!host.path("cli-argv.txt").exists(), "{device:?}");
        }
    }
    let host = Host::new("fake-runtime");
    let output = std::process::Command::new(workspace_file(CREATE))
        .arg("--state-dir")
        .arg(host.path("state"))
        .arg("--device")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--device needs a value"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_allowlisted_device_is_forwarded_once_and_reported_in_the_metadata() {
    let host = Host::new("fake-runtime");
    host.device_is(Path::new("/dev/null"));
    let output = host.run(
        "fake-runtime",
        &["--device", "/dev/kvm", "--device", "/dev/kvm"],
        false,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let argv = host.run_argv();
    assert_eq!(device_pairs(&argv), ["/dev/kvm"], "{argv:?}");
    assert!(!argv.iter().any(|arg| arg == "keep-groups"), "{argv:?}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["metadata"]["devices"],
        serde_json::json!(["/dev/kvm"])
    );
}

#[test]
fn no_device_requested_forwards_none_and_reports_an_empty_list() {
    let host = Host::new("fake-runtime");
    let output = host.run("fake-runtime", &[], false);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(device_pairs(&host.run_argv()).is_empty());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["metadata"]["devices"], serde_json::json!([]));

    let output = host.run("fake-runtime", &[], true);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(device_check(&output), ["ok", "device", "not requested", ""]);
}

#[test]
fn the_device_check_reports_a_usable_device_before_the_last_state_dir_check() {
    let host = Host::new("fake-runtime");
    host.device_is(Path::new("/dev/null"));
    let output = host.run("fake-runtime", &["--device", "/dev/kvm"], true);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        device_check(&output),
        [
            "ok",
            "device",
            "device /dev/kvm is present and this user can read and write it",
            ""
        ]
    );
    let names: Vec<String> = checks(&output).into_iter().map(|f| f[1].clone()).collect();
    assert_eq!(
        names.last().map(String::as_str),
        Some("state-dir"),
        "{names:?}"
    );
}

#[test]
fn a_missing_device_fails_the_preflight_and_the_create_with_its_own_exit_code() {
    let host = Host::new("fake-runtime");
    let output = host.run("fake-runtime", &["--device", "/dev/kvm"], true);
    assert_eq!(output.status.code(), Some(1));
    let check = device_check(&output);
    assert_eq!(check[0], "fail");
    assert_eq!(check[2], "device /dev/kvm is not present on this host");
    assert_eq!(
        check[3],
        "enable KVM on this host (virtualisation on in the firmware, the kvm module loaded), or remove --device /dev/kvm from the container config"
    );

    let output = host.run("fake-runtime", &["--device", "/dev/kvm"], false);
    assert_eq!(output.status.code(), Some(9), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("device /dev/kvm is not present on this host"),
        "{}",
        stderr(&output)
    );
    assert!(host.state_entries().is_empty());
    assert!(!host.path("cli-argv.txt").exists());

    // A regular file is not a device.
    let host = Host::new("fake-runtime");
    let file = host.path("not-a-device");
    fs::write(&file, "").unwrap();
    host.device_is(&file);
    let output = host.run("fake-runtime", &["--device", "/dev/kvm"], true);
    assert_eq!(device_check(&output)[0], "fail");
}

/// A character device the invoking user cannot both read and write, if
/// the host has one (none for root).
fn unusable_char_device() -> Option<PathBuf> {
    use std::os::unix::fs::FileTypeExt;
    [
        "/dev/mem",
        "/dev/port",
        "/dev/console",
        "/dev/kmsg",
        "/dev/tty0",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| {
        fs::metadata(path).is_ok_and(|meta| meta.file_type().is_char_device())
            && fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
    })
}

#[test]
fn a_device_this_user_cannot_read_and_write_fails_with_the_group_remedy() {
    let Some(device) = unusable_char_device() else {
        eprintln!("skipped: no character device on this host is closed to this user");
        return;
    };
    let host = Host::new("fake-runtime");
    host.device_is(&device);
    let output = host.run("fake-runtime", &["--device", "/dev/kvm"], true);
    assert_eq!(output.status.code(), Some(1));
    let check = device_check(&output);
    assert_eq!(check[0], "fail");
    assert_eq!(
        check[2],
        "device /dev/kvm is present but this user cannot read and write it"
    );
    assert_eq!(
        check[3],
        "add the user to the kvm group (sudo usermod -aG kvm \"$USER\", then log in again), or remove --device /dev/kvm from the container config"
    );
    let output = host.run("fake-runtime", &["--device", "/dev/kvm"], false);
    assert_eq!(output.status.code(), Some(9), "{}", stderr(&output));
    assert!(host.state_entries().is_empty());
}

#[test]
fn a_world_read_write_device_needs_no_group_under_podman_or_docker() {
    // edinburgh's /dev/kvm is 0666: the container's user (the same uid)
    // opens it through the other bits, so no group is added.
    for cli in ["podman", "docker"] {
        let host = Host::new(cli);
        host.device_is(Path::new("/dev/null"));
        let output = host.run(cli, &["--device", "/dev/kvm"], false);
        assert!(output.status.success(), "{cli}: {}", stderr(&output));
        let argv = host.run_argv();
        assert_eq!(device_pairs(&argv), ["/dev/kvm"], "{cli}");
        assert!(group_adds(&argv).is_empty(), "{cli}: {argv:?}");
    }
}

fn group_adds(argv: &[String]) -> Vec<&str> {
    argv.windows(2)
        .filter(|pair| pair[0] == "--group-add")
        .map(|pair| pair[1].as_str())
        .collect()
}

/// This user's uid and a group it is in, and a gid it is not in.
fn ids() -> (u32, u32, u32) {
    let id = |flag: &str| -> Vec<u32> {
        let out = std::process::Command::new("id").arg(flag).output().unwrap();
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .map(|id| id.parse().unwrap())
            .collect()
    };
    let groups = id("-G");
    let outside = (4_000_000..).find(|gid| !groups.contains(gid)).unwrap();
    (id("-u")[0], groups[0], outside)
}

/// The preflight (exit 1 on a failure) and the create (the check's own
/// exit code) for a device `stat` describes as `line`, under `cli`.
fn judge(cli: &str, line: &str, host: Host) -> (Vec<String>, Output, Host) {
    let host = host.with_device_stat(line);
    let preflight = host.run(cli, &["--device", "/dev/kvm"], true);
    let check = device_check(&preflight);
    let create = host.run(cli, &["--device", "/dev/kvm"], false);
    (check, create, host)
}

const USABLE: &str = "device /dev/kvm is present and this user can read and write it";
const ACL: &str = "device /dev/kvm is readable and writable here only through an ACL or a capability, which the container does not keep";

#[test]
fn owner_or_other_bits_pass_as_they_are_under_every_runtime() {
    let (uid, _, _) = ids();
    let other = uid + 1;
    for line in [
        "666 0 0 root".to_string(),
        format!("66 {other} 0 root"),
        format!("6 {other} 0 root"),
        format!("600 {uid} 0 root"),
    ] {
        for cli in ["podman", "docker", "fake-runtime"] {
            let (check, create, host) = judge(cli, &line, Host::new(cli));
            assert_eq!(check, ["ok", "device", USABLE, ""], "{line} {cli}");
            assert_eq!(
                create.status.code(),
                Some(0),
                "{line} {cli}: {}",
                stderr(&create)
            );
            let argv = host.run_argv();
            assert_eq!(device_pairs(&argv), ["/dev/kvm"], "{line} {cli}");
            assert!(group_adds(&argv).is_empty(), "{line} {cli}: {argv:?}");
        }
    }
}

#[test]
fn access_that_would_not_survive_into_the_container_is_refused() {
    let (uid, _, outside) = ids();
    let other = uid + 1;
    // Group bits for a group this user is not in, an owner-only device of
    // someone else, no bits at all: -r/-w pass here only through an ACL or
    // a capability.
    for line in [
        format!("660 {other} {outside} nobody"),
        format!("600 {other} 0 root"),
        "0 0 0 root".to_string(),
    ] {
        let (check, create, _) = judge("docker", &line, Host::new("docker"));
        assert_eq!(check[0], "fail", "{line}");
        assert_eq!(check[2], ACL, "{line}");
        assert_eq!(
            check[3],
            "make /dev/kvm read-write for a group the user is in (sudo usermod -aG kvm \"$USER\") or for everyone (0666), or remove --device /dev/kvm from the container config"
        );
        assert_eq!(create.status.code(), Some(9), "{line}");
    }
    // Anything stat cannot be read as is refused the same way.
    let (check, create, _) = judge("docker", "garbage", Host::new("docker"));
    assert_eq!(check[2], ACL);
    assert_eq!(create.status.code(), Some(9));
}

#[test]
fn group_only_access_keeps_just_that_group_under_docker() {
    let (uid, gid, _) = ids();
    let other = uid + 1;
    for mode in ["660", "2660", "60"] {
        let line = format!("{mode} {other} {gid} kvm");
        let (check, create, host) = judge("docker", &line, Host::new("docker"));
        assert_eq!(
            check,
            [
                "ok".to_string(),
                "device".to_string(),
                format!(
                    "{USABLE} through group kvm (gid {gid}); docker runs the container with --group-add {gid}"
                ),
                String::new()
            ],
            "{mode}"
        );
        assert_eq!(create.status.code(), Some(0), "{}", stderr(&create));
        assert_eq!(group_adds(&host.run_argv()), [gid.to_string()]);
    }
    // A group without a name is shown by its id alone.
    let line = format!("660 {other} {gid} UNKNOWN");
    let (check, _, _) = judge("docker", &line, Host::new("docker"));
    assert_eq!(
        check[2],
        format!("{USABLE} through gid {gid}; docker runs the container with --group-add {gid}")
    );
}

#[test]
fn group_only_access_under_podman_keeps_the_users_groups_with_crun_alone() {
    let (uid, gid, _) = ids();
    let other = uid + 1;
    let line = format!("660 {other} {gid} kvm");
    let (check, create, host) = judge("podman", &line, Host::new("podman"));
    assert_eq!(
        check[2],
        format!(
            "{USABLE} through group kvm (gid {gid}); podman keeps the user's groups (--group-add keep-groups, crun)"
        )
    );
    assert_eq!(create.status.code(), Some(0), "{}", stderr(&create));
    assert_eq!(group_adds(&host.run_argv()), ["keep-groups"]);

    let (check, create, host) = judge(
        "podman",
        &line,
        Host::new("podman").with_oci_runtime("runc"),
    );
    assert_eq!(check[0], "fail");
    assert_eq!(
        check[2],
        format!(
            "device /dev/kvm is readable and writable only through group kvm (gid {gid}), and podman's OCI runtime is runc, not crun: the container would not keep that group"
        )
    );
    assert_eq!(
        check[3],
        "install crun and set runtime = \"crun\" under [engine] in containers.conf, or make /dev/kvm read-write for everyone (0666), or remove --device /dev/kvm from the container config"
    );
    assert_eq!(create.status.code(), Some(9));
    assert!(host.state_entries().is_empty());
}

#[test]
fn a_podman_that_cannot_name_its_runtime_is_a_runtime_failure() {
    let (uid, gid, _) = ids();
    let other = uid + 1;
    let line = format!("660 {other} {gid} kvm");
    for (host, detail) in [
        (
            Host::new("podman").with_oci_exit(125),
            "podman could not report its OCI runtime (exit 125)",
        ),
        (
            Host::new("podman").with_oci_runtime(""),
            "podman could not report its OCI runtime (exit 0)",
        ),
        (
            Host::new("podman").with_oci_exit(124),
            "podman did not answer within 15s while reporting its OCI runtime",
        ),
    ] {
        let (check, create, _) = judge("podman", &line, host);
        assert_eq!(check[0], "fail", "{detail}");
        assert_eq!(check[2], detail);
        assert!(check[3].contains("(podman info)"), "{check:?}");
        assert_eq!(create.status.code(), Some(3), "{detail}");
    }
}

#[test]
fn a_runtime_that_is_neither_podman_nor_docker_cannot_carry_a_group() {
    let (uid, gid, _) = ids();
    let other = uid + 1;
    let line = format!("660 {other} {gid} kvm");
    let (check, create, _) = judge("fake-runtime", &line, Host::new("fake-runtime"));
    assert_eq!(check[0], "fail");
    assert_eq!(
        check[2],
        format!("cannot judge whether fake-runtime keeps group kvm (gid {gid})")
    );
    assert_eq!(check[3], "make /dev/kvm 0666 or use podman/docker");
    assert_eq!(create.status.code(), Some(9));
    assert!(
        stderr(&create).contains(&format!(
            "cannot judge whether fake-runtime keeps group kvm (gid {gid}); make /dev/kvm 0666 or use podman/docker"
        )),
        "{}",
        stderr(&create)
    );
}

#[test]
fn podman_named_by_path_is_still_podman() {
    let (uid, gid, _) = ids();
    let other = uid + 1;
    let line = format!("660 {other} {gid} kvm");
    for name in ["podman", "podman-remote"] {
        let host = Host::new(name);
        let cli = host.path(&format!("bin/{name}")).display().to_string();
        let (check, create, host) = judge(&cli, &line, host);
        assert!(
            check[2].ends_with("podman keeps the user's groups (--group-add keep-groups, crun)"),
            "{name}: {check:?}"
        );
        assert_eq!(create.status.code(), Some(0), "{}", stderr(&create));
        let argv = host.run_argv();
        assert_eq!(group_adds(&argv), ["keep-groups"], "{name}");
        assert!(
            argv.iter().any(|arg| arg == "--userns=keep-id"),
            "{name}: {argv:?}"
        );
    }
}

#[test]
fn without_a_runtime_the_group_is_a_warning_not_a_verdict() {
    let (uid, gid, _) = ids();
    let other = uid + 1;
    let host = Host::new("podman").with_device_stat(&format!("660 {other} {gid} kvm"));
    let output = host.run("no-such-runtime-cli", &["--device", "/dev/kvm"], true);
    let check = device_check(&output);
    assert_eq!(
        check,
        [
            "warn".to_string(),
            "device".to_string(),
            format!(
                "{USABLE} through group kvm (gid {gid}); whether the container keeps that group was not checked: no container runtime"
            ),
            "fix the runtime-cli check first".to_string()
        ]
    );
}

/// The script's allowlist and init's are one list, and the script's own
/// guard admits exactly it.
#[test]
fn the_scripts_allowlist_is_the_harness_allowlist() {
    let create = fs::read_to_string(workspace_file(CREATE)).unwrap();
    let definition = create
        .lines()
        .find(|line| line.starts_with("allowed_devices=("))
        .expect("allowed_devices=(...) on one line");
    let listed: Vec<&str> = definition
        .strip_prefix("allowed_devices=(")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap()
        .split_whitespace()
        .collect();
    assert_eq!(
        listed,
        quecto::application::environments::dto::ALLOWED_CONTAINER_DEVICES
    );
    // The list is read in one place: the guard.
    let start = create.find("is_allowed_device() {").unwrap();
    let end = start + create[start..].find("\n}\n").unwrap() + 3;
    let guard = &create[start..end];
    assert_eq!(create.matches("allowed_devices").count(), 2);
    assert_eq!(guard.matches("allowed_devices").count(), 1);
    // And the guard admits exactly the list, spelled exactly.
    let spellings = [
        "/dev/kvm",
        "/dev/kvm/",
        "/dev//kvm",
        "//dev/kvm",
        "/dev/./kvm",
        "/dev/../dev/kvm",
        "dev/kvm",
        "kvm",
        "/dev/KVM",
        " /dev/kvm",
        "/dev/kvm ",
        "/dev/kvm\n",
        "/dev/kvm*",
        "/dev/k?m",
        "/dev/sda",
        "/dev/kvm:/dev/kvm",
        "",
    ];
    let mut script = format!("{definition}\n{guard}");
    script.push_str(
        "for s in \"$@\"; do if is_allowed_device \"$s\"; then echo yes; else echo no; fi; done\n",
    );
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(&script)
        .arg("guard")
        .args(spellings)
        .output()
        .unwrap();
    let verdicts: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let expected: Vec<&str> = spellings
        .iter()
        .map(|s| if listed.contains(s) { "yes" } else { "no" })
        .collect();
    assert_eq!(verdicts, expected);
}

#[test]
fn the_production_script_reads_no_device_environment_variable() {
    let create = fs::read_to_string(workspace_file(CREATE)).unwrap();
    assert!(!create.contains("QUECTO_CONTAINER_DEVICE"));
}
