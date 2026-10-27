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
    /// What the fake podman says its OCI runtime is.
    oci_runtime: &'static str,
}

impl Host {
    fn new(cli_name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        write_executable(
            &bin.join(cli_name),
            format!(
                "#!/usr/bin/env bash\nif [ \"$1\" = info ]; then printf '%s\\n' \"$FAKE_OCI_RUNTIME\"; fi\nif [ \"$1\" = run ] && [ \"$2\" != --rm ]; then printf '%s\\n' \"$@\" > '{}'; fi\nexit 0\n",
                dir.path().join("cli-argv.txt").display()
            ),
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
        }
    }

    fn with_oci_runtime(mut self, runtime: &'static str) -> Self {
        self.oci_runtime = runtime;
        self
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
            .env("FAKE_OCI_RUNTIME", self.oci_runtime)
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

/// A character device this user reaches through one of its groups alone
/// (group read-write; neither world read-write nor owned read-write by
/// this user), with that group's id — e.g. /dev/ttyS* (uucp) or
/// /dev/uinput (input). `None` when the host has none, or for root.
fn group_only_char_device() -> Option<(PathBuf, u32)> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let ids = |flag: &str| -> Vec<u32> {
        let out = std::process::Command::new("id").arg(flag).output().unwrap();
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .filter_map(|id| id.parse().ok())
            .collect()
    };
    let uid = *ids("-u").first()?;
    let groups = ids("-G");
    if uid == 0 {
        return None;
    }
    let mut entries: Vec<PathBuf> = fs::read_dir("/dev")
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    entries.sort();
    entries.into_iter().find_map(|path| {
        let meta = fs::metadata(&path).ok()?;
        let mode = meta.mode();
        let group_only = meta.file_type().is_char_device()
            && mode & 0o060 == 0o060
            && mode & 0o006 != 0o006
            && meta.uid() != uid
            && groups.contains(&meta.gid());
        group_only.then(|| (path, meta.gid()))
    })
}

#[test]
fn a_group_only_device_keeps_the_group_under_podman_with_crun_and_adds_it_under_docker() {
    let Some((device, gid)) = group_only_char_device() else {
        eprintln!("skipped: no character device on this host is reachable through a group alone");
        return;
    };
    let through = format!("(gid {gid})");

    let host = Host::new("podman");
    host.device_is(&device);
    let output = host.run("podman", &["--device", "/dev/kvm"], true);
    assert!(output.status.success(), "{}", stderr(&output));
    let check = device_check(&output);
    assert_eq!(check[0], "ok");
    assert!(
        check[2].starts_with(
            "device /dev/kvm is present and this user can read and write it through group "
        ) && check[2].contains(&through)
            && check[2]
                .ends_with("; podman keeps the user's groups (--group-add keep-groups, crun)"),
        "{check:?}"
    );
    let output = host.run("podman", &["--device", "/dev/kvm"], false);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(group_adds(&host.run_argv()), ["keep-groups"]);

    let host = Host::new("podman").with_oci_runtime("runc");
    host.device_is(&device);
    let output = host.run("podman", &["--device", "/dev/kvm"], true);
    assert_eq!(output.status.code(), Some(1));
    let check = device_check(&output);
    assert_eq!(check[0], "fail");
    assert!(
        check[2].starts_with("device /dev/kvm is readable and writable only through group ")
            && check[2].contains(&through)
            && check[2].ends_with(
                ", and podman's OCI runtime is runc, not crun: the container would not keep that group"
            ),
        "{check:?}"
    );
    assert_eq!(
        check[3],
        "install crun and set runtime = \"crun\" under [engine] in containers.conf, or make /dev/kvm read-write for everyone (0666), or remove --device /dev/kvm from the container config"
    );
    let output = host.run("podman", &["--device", "/dev/kvm"], false);
    assert_eq!(output.status.code(), Some(9), "{}", stderr(&output));
    assert!(host.state_entries().is_empty());

    let host = Host::new("docker");
    host.device_is(&device);
    let output = host.run("docker", &["--device", "/dev/kvm"], false);
    assert!(output.status.success(), "{}", stderr(&output));
    let argv = host.run_argv();
    assert_eq!(group_adds(&argv), [gid.to_string()], "{argv:?}");
    let output = host.run("docker", &["--device", "/dev/kvm"], true);
    let check = device_check(&output);
    assert!(
        check[2].ends_with(&format!(
            "; docker runs the container with --group-add {gid}"
        )),
        "{check:?}"
    );
}

/// The script's allowlist and init's are one list.
#[test]
fn the_scripts_allowlist_is_the_harness_allowlist() {
    let create = fs::read_to_string(workspace_file(CREATE)).unwrap();
    let lines: Vec<&str> = create
        .lines()
        .filter(|line| line.starts_with("allowed_devices=("))
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let listed: Vec<&str> = lines[0]
        .strip_prefix("allowed_devices=(")
        .and_then(|rest| rest.strip_suffix(')'))
        .expect("allowed_devices=(...) on one line")
        .split_whitespace()
        .collect();
    assert_eq!(
        listed,
        quecto::application::environments::dto::ALLOWED_CONTAINER_DEVICES
    );
}

#[test]
fn the_production_script_has_no_device_test_seam() {
    let create = fs::read_to_string(workspace_file(CREATE)).unwrap();
    assert!(!create.contains("QUECTO_CONTAINER_DEVICE_ROOT"));
}
