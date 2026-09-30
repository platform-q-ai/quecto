//! Real adapter confinement contracts, ported from the Python fixtures.
#![cfg(unix)]
use std::{
    fs,
    os::unix::{
        fs::{PermissionsExt, symlink},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};

fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn mode(path: &Path, bits: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(bits)).unwrap();
}
fn adapter() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    [
        ".quecto/containers/standard/create.sh",
        ".quecto/containers/standard/scripts/create.sh",
    ]
    .iter()
    .map(|p| root.join(p))
    .find(|p| p.is_file())
    .expect("project adapter")
}
struct Fixture {
    scratch: tempfile::TempDir,
    base: PathBuf,
    bin: PathBuf,
    home: PathBuf,
    timeout: Duration,
}
impl Fixture {
    fn new() -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let base = root.join("base");
        fs::create_dir(&base).unwrap();
        mode(&base, 0o700);
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        let home = root.join("home");
        fs::create_dir_all(home.join(".quecto")).unwrap();
        fs::write(
            home.join(".gitconfig"),
            "[user]\nname = Fixture\nemail = fixture@example.invalid\n",
        )
        .unwrap();
        executable(
            &bin.join("probe"),
            &format!(
                "#!/bin/bash -p\n/usr/bin/env | cut -d= -f1 | sort > '{}'\n",
                root.join("names").display()
            ),
        );
        executable(&bin.join("gh"), "#!/bin/sh\nprintf '%s\\n' fixture\n");
        executable(
            &bin.join("podman"),
            &format!(
                r#"#!/bin/bash -p
set -eu
if [ "$1" = image ]; then case "$2" in exists) exit 0;; inspect) printf '\n'; exit 0;; esac; fi
if [ "$1" = run ] && [ "$2" = --rm ]; then exit 0; fi
if [ "$1" = run ]; then
 shift
 runtime_env=(PATH=/usr/bin:/bin UNRECOGNISED_IMAGE_CREDENTIAL=fixture)
 while [ "$#" -gt 0 ]; do
  case "$1" in
   -v|--volume) printf '%s\n' "$2" >> '{}'; shift 2;;
   -e|--env) runtime_env+=("$2"); shift 2;;
   --name|--hostname|--label|--user|-w|--workdir|--pids-limit) shift 2;;
   --*|-d) shift;;
   fixture-image) shift; break;;
   *) exit 125;;
  esac
 done
 env -i "${{runtime_env[@]}}" "$@"
 printf 'fixture-container\n'
 exit 0
fi
exit 125
"#,
                root.join("mounts").display()
            ),
        );
        Self {
            scratch,
            base,
            bin,
            home,
            timeout: Duration::from_secs(15),
        }
    }
    fn root(&self) -> &Path {
        self.scratch.path()
    }
    fn cargo(&self) -> PathBuf {
        let p = self.base.join("rust-cache/cargo");
        fs::create_dir_all(&p).unwrap();
        mode(p.parent().unwrap(), 0o700);
        mode(&p, 0o700);
        p
    }
    fn launch(&self, base: &Path, declared: &Path, agent: bool) -> Output {
        let mut cmd = Command::new(adapter());
        cmd.env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env("HOME", &self.home)
            .env("QUECTO_BASE_DIR", declared)
            .env("QUECTO_CONTAINER_CLI", "podman")
            .env("QUECTO_CONTAINER_ENVIRONMENT_REF", "standard")
            .env("QUECTO_REPO_CHECK_TIMEOUT", "1")
            .env("ANTHROPIC_API_KEY", "fixture")
            .env("OPENAI_API_KEY", "fixture")
            .env("UNRECOGNISED_PROVIDER_CREDENTIAL", "fixture")
            .arg("--state-dir")
            .arg(base.join("container-environments"))
            .args(["--image", "fixture-image", "--"])
            .arg(self.bin.join("probe"));
        if agent {
            cmd.args(["agent", "--mode", "uds"]);
        }
        cmd.arg("--socket").arg(self.root().join("child.sock"));
        bounded_output(cmd, self.timeout)
    }
    fn run(&self) -> Output {
        self.launch(&self.base, &self.base, false)
    }
}
fn bounded_output(mut cmd: Command, timeout: Duration) -> Output {
    assert!(timeout > Duration::ZERO);
    let out = tempfile::tempfile().unwrap();
    let err = tempfile::tempfile().unwrap();
    cmd.stdout(out.try_clone().unwrap())
        .stderr(err.try_clone().unwrap())
        .process_group(0);
    let mut child = cmd.spawn().unwrap();
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        } else {
            let killed = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .status()
                .unwrap();
            let _ = child.wait();
            assert!(killed.success(), "must kill entire fixture process group");
            panic!("adapter fixture exceeded hard deadline");
        }
    };
    use std::io::{Read, Seek};
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut out = out;
    let mut err = err;
    out.rewind().unwrap();
    err.rewind().unwrap();
    out.read_to_end(&mut stdout).unwrap();
    err.read_to_end(&mut stderr).unwrap();
    Output {
        status,
        stdout,
        stderr,
    }
}
fn code(out: Output, expected: i32) {
    assert_eq!(
        out.status.code(),
        Some(expected),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn cargo_install_manifests_are_accepted_on_reuse() {
    let f = Fixture::new();
    let cargo = f.cargo();
    fs::write(
        cargo.join(".crates.toml"),
        "[v1]\n\"sccache 0.10.0\" = [\"sccache\"]\n",
    )
    .unwrap();
    fs::write(cargo.join(".crates2.json"), "{\"installs\": {}}\n").unwrap();
    code(f.run(), 0);
}
#[test]
fn writable_cache_base_is_refused_without_chmod() {
    let f = Fixture::new();
    mode(&f.base, 0o777);
    code(f.run(), 8);
    assert!(!f.base.join("rust-cache").exists());
    assert_eq!(
        fs::metadata(&f.base).unwrap().permissions().mode() & 0o777,
        0o777
    );
}
#[test]
fn writable_nonsticky_namespace_parent_is_refused() {
    let f = Fixture::new();
    mode(f.root(), 0o777);
    code(f.run(), 8);
    assert!(!f.base.join("rust-cache").exists());
    assert_eq!(
        fs::metadata(f.root()).unwrap().permissions().mode() & 0o777,
        0o777
    );
}
#[test]
fn simultaneous_cold_creates_both_succeed() {
    let f = Fixture::new();
    let barrier = f.root().join("barrier");
    fs::create_dir(&barrier).unwrap();
    executable(
        &f.bin.join("mkdir"),
        &format!(
            r#"#!/bin/bash -p
set -eu
if [ "${{@: -1}}" = '{}' ]; then
 /usr/bin/touch '{}/'"$$"
 for attempt in {{1..200}}; do
  files=('{}/'*)
  if [ "${{#files[@]}}" = 2 ]; then break; fi
  /usr/bin/sleep 0.01
 done
 files=('{}/'*)
 [ "${{#files[@]}}" = 2 ] || exit 124
fi
exec /usr/bin/mkdir "$@"
"#,
            f.base.join("rust-cache").display(),
            barrier.display(),
            barrier.display(),
            barrier.display()
        ),
    );
    std::thread::scope(|s| {
        let a = s.spawn(|| f.run());
        let b = s.spawn(|| f.run());
        code(a.join().unwrap(), 0);
        code(b.join().unwrap(), 0);
    });
    assert_eq!(fs::read_dir(barrier).unwrap().count(), 2);
    for p in ["rust-cache", "rust-cache/cargo", "rust-cache/sccache"] {
        assert_eq!(
            fs::metadata(f.base.join(p)).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
#[test]
fn legitimate_base_symlink_resolves_to_authoritative_parent() {
    let f = Fixture::new();
    let alias = f.root().join("alias");
    symlink(&f.base, &alias).unwrap();
    code(f.launch(&alias, &alias, false), 0);
    let mounts = fs::read_to_string(f.root().join("mounts")).unwrap();
    for m in mounts.lines().filter(|m| m.contains(":/tmp/quecto-")) {
        assert!(m.contains(f.base.join("rust-cache").to_str().unwrap()));
    }
}
#[test]
fn override_cannot_redirect_cache_authority() {
    let f = Fixture::new();
    let other = f.root().join("other");
    fs::create_dir(&other).unwrap();
    code(f.launch(&f.base, &other, false), 8);
    assert!(!other.join("rust-cache").exists());
}
#[test]
fn cache_symlink_is_refused() {
    let f = Fixture::new();
    let outside = f.root().join("outside");
    fs::create_dir(&outside).unwrap();
    mode(&outside, 0o700);
    symlink(&outside, f.base.join("rust-cache")).unwrap();
    code(f.run(), 8);
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}
#[test]
fn child_cache_symlink_is_refused() {
    let f = Fixture::new();
    f.cargo();
    let outside = f.root().join("outside");
    fs::create_dir(&outside).unwrap();
    mode(&outside, 0o700);
    symlink(&outside, f.base.join("rust-cache/sccache")).unwrap();
    code(f.run(), 8);
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}
#[test]
fn hidden_and_visible_configuration_refused_without_contents() {
    for name in [
        "config.toml",
        ".config.toml",
        "credentials.toml",
        ".credentials",
    ] {
        let f = Fixture::new();
        fs::write(f.cargo().join(name), "fixture-content-must-stay-private").unwrap();
        let r = f.run();
        assert_eq!(r.status.code(), Some(8));
        assert!(String::from_utf8_lossy(&r.stderr).contains("non-allowlisted entry"));
        assert!(!String::from_utf8_lossy(&r.stdout).contains("fixture-content-must-stay-private"));
        assert!(!String::from_utf8_lossy(&r.stderr).contains("fixture-content-must-stay-private"));
    }
}
#[test]
fn insecure_existing_cache_refused_without_chmod() {
    let f = Fixture::new();
    f.cargo();
    let root = f.base.join("rust-cache");
    mode(&root, 0o755);
    code(f.run(), 8);
    assert_eq!(
        fs::metadata(root).unwrap().permissions().mode() & 0o777,
        0o755
    );
}
#[test]
fn cargo_configuration_is_refused() {
    let f = Fixture::new();
    fs::write(f.cargo().join("credentials.toml"), "fixture").unwrap();
    code(f.run(), 8);
}
#[test]
fn allowed_entry_symlink_escape_is_refused() {
    let f = Fixture::new();
    symlink(f.root(), f.cargo().join("registry")).unwrap();
    code(f.run(), 8);
}
#[test]
fn build_child_only_receives_allowlisted_names() {
    let f = Fixture::new();
    code(f.run(), 0);
    let names = fs::read_to_string(f.root().join("names")).unwrap();
    let allowed = [
        "PATH",
        "HOME",
        "CARGO_HOME",
        "SCCACHE_DIR",
        "SCCACHE_CACHE_SIZE",
        "RUSTC_WRAPPER",
        "LC_CTYPE",
        "PWD",
        "SHLVL",
        "TERM",
    ];
    for n in names.lines() {
        assert!(allowed.contains(&n), "unexpected build name {n}");
    }
    let boundary = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".quecto/containers/standard/build-environment.sh")
        .canonicalize()
        .unwrap();
    assert!(
        fs::read_to_string(f.root().join("mounts"))
            .unwrap()
            .lines()
            .any(|l| l == format!("{}:{}:ro", boundary.display(), boundary.display()))
    );
}
#[test]
fn supported_agent_protocol_preserves_authentication() {
    let f = Fixture::new();
    code(f.launch(&f.base, &f.base, true), 0);
    let names = fs::read_to_string(f.root().join("names")).unwrap();
    let names: Vec<_> = names.lines().collect();
    for required in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "QUECTO_SWARM_BOOTSTRAP",
    ] {
        assert!(names.contains(&required));
    }
    let mut allowed: Vec<String> = [
        "PATH",
        "HOME",
        "CARGO_HOME",
        "SCCACHE_DIR",
        "SCCACHE_CACHE_SIZE",
        "RUSTC_WRAPPER",
        "LC_CTYPE",
        "PWD",
        "SHLVL",
        "RUST_LOG",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "QUECTO_SWARM_BOOTSTRAP",
        "QUECTO_SWARM_CONTAINER",
        "QUECTO_SWARM_HOST_PID_NS",
        "QUECTO_SWARM_CHECKOUT",
        "GIT_CONFIG_COUNT",
        "UNRECOGNISED_IMAGE_CREDENTIAL",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for i in 0..3 {
        allowed.push(format!("GIT_CONFIG_KEY_{i}"));
        allowed.push(format!("GIT_CONFIG_VALUE_{i}"));
    }
    for name in names {
        assert!(
            allowed.iter().any(|a| a == name),
            "unexpected agent name {name}"
        );
    }
}

#[test]
fn timeout_kills_process_group_and_cleanup_survives_unwind() {
    let start = Instant::now();
    let scratch_path;
    {
        let mut f = Fixture::new();
        scratch_path = f.root().to_path_buf();
        f.timeout = Duration::from_millis(100);
        executable(
            &f.bin.join("podman"),
            "#!/bin/bash -p\ntrap '' TERM\nsleep 60 &\nwait\n",
        );
        let result = std::panic::catch_unwind(|| f.run());
        assert!(result.is_err(), "unbounded fixture must fail explicitly");
    }
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(
        !scratch_path.exists(),
        "scratch must be removed after timeout"
    );
}

#[test]
fn writable_state_namespace_is_refused_without_chmod() {
    let f = Fixture::new();
    let state = f.base.join("container-environments");
    fs::create_dir(&state).unwrap();
    mode(&state, 0o777);
    code(f.run(), 8);
    assert_eq!(
        fs::metadata(state).unwrap().permissions().mode() & 0o777,
        0o777
    );
}

#[test]
fn state_leaf_symlink_is_refused() {
    let f = Fixture::new();
    let outside = f.root().join("outside");
    fs::create_dir(&outside).unwrap();
    mode(&outside, 0o700);
    symlink(&outside, f.base.join("container-environments")).unwrap();
    code(f.run(), 8);
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn sticky_namespace_ancestor_protecting_owned_child_is_accepted() {
    let f = Fixture::new();
    mode(f.root(), 0o1777);
    code(f.run(), 0);
}

#[test]
fn sticky_writable_cache_base_is_refused() {
    let f = Fixture::new();
    mode(&f.base, 0o1777);
    code(f.run(), 8);
    assert!(!f.base.join("rust-cache").exists());
}

#[test]
fn cargo_install_manifest_symlinks_are_refused() {
    for name in [".crates.toml", ".crates2.json"] {
        let f = Fixture::new();
        let outside = f.root().join("outside-manifest");
        fs::write(&outside, "fixture-private-manifest").unwrap();
        symlink(&outside, f.cargo().join(name)).unwrap();
        let result = f.run();
        assert_eq!(result.status.code(), Some(8));
        assert!(!String::from_utf8_lossy(&result.stderr).contains("fixture-private-manifest"));
    }
}

#[test]
fn cargo_install_manifest_directories_are_refused() {
    for name in [".crates.toml", ".crates2.json"] {
        let f = Fixture::new();
        fs::create_dir(f.cargo().join(name)).unwrap();
        code(f.run(), 8);
    }
}

#[test]
fn real_offline_cargo_install_is_accepted_on_second_launch() {
    let f = Fixture::new();
    code(f.run(), 0);
    let package = f.root().join("package");
    fs::create_dir_all(package.join("src")).unwrap();
    fs::write(package.join("Cargo.toml"), "[package]\nname = \"cache-manifest-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n").unwrap();
    fs::write(package.join("src/main.rs"), "fn main() {}\n").unwrap();
    let tool_path = std::env::var_os("PATH").expect("installed Rust toolchain PATH");
    let mut discovery = Command::new("rustup");
    discovery.env_clear().env("PATH", &tool_path).env("HOME", &f.home)
        .env("RUSTUP_HOME", std::env::var_os("RUSTUP_HOME").expect("installed rustup home"))
        .args(["which", "cargo"]);
    let located = bounded_output(discovery, Duration::from_secs(5));
    assert!(located.status.success(), "{}", String::from_utf8_lossy(&located.stderr));
    let cargo_binary = PathBuf::from(String::from_utf8(located.stdout).unwrap().trim());
    assert!(cargo_binary.is_file());
    let compiler_path = format!("{}:/usr/bin:/bin", cargo_binary.parent().unwrap().display());
    let mut install = Command::new(&cargo_binary);
    install
        .env_clear()
        .env("PATH", compiler_path)
        .env("HOME", &f.home)
        .env("CARGO_HOME", f.root().join("installer-home"))
        .env("CARGO_TARGET_DIR", f.root().join("install-target"))
        .args(["install", "--offline", "--path"])
        .arg(&package)
        .arg("--root")
        .arg(f.base.join("rust-cache/cargo"));
    code(bounded_output(install, Duration::from_secs(30)), 0);
    let cargo = f.base.join("rust-cache/cargo");
    assert!(cargo.join(".crates.toml").is_file());
    assert!(cargo.join(".crates2.json").is_file());
    assert!(cargo.join("bin/cache-manifest-fixture").is_file());
    code(f.run(), 0);
}
