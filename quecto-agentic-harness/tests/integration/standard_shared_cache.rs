//! #2383: the project-owned standard adapter, not the tooling-neutral starter.
//! Before its opt-in script exists, the shipped adapter is the explicitly
//! identified baseline. Only absence permits that fallback; script failures do
//! not. This makes the initial red a successful launch lacking cache behavior,
//! rather than a missing-file or unknown-argument failure.
//! The child here is an explicit build probe, NOT an agent. The secret-free
//! assertion must not be implemented by removing credentials from normal agents;
//! delivery must keep agent auth separate from cache-using builds/server env.
//! Cargo-home entry names are a topology/provenance check, not content scanning.
#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn project_adapter() -> Option<PathBuf> {
    [
        ".quecto/containers/standard/create.sh",
        ".quecto/containers/standard/scripts/create.sh",
    ]
    .into_iter()
    .map(|relative| root().join(relative))
    .find(|path| path.is_file())
}

#[test]
fn standard_definition_owns_cache_opt_in_adapter() {
    assert!(
        project_adapter().is_some(),
        "standard definition has no project-owned create opt-in; generic starter must stay tooling-neutral"
    );
}

fn image_env() -> BTreeMap<String, String> {
    let image =
        fs::read_to_string(root().join(".quecto/containers/standard/Containerfile")).unwrap();
    let joined = image.replace("\\\n", " ");
    let mut env = BTreeMap::new();
    for line in joined
        .lines()
        .filter_map(|line| line.trim().strip_prefix("ENV "))
    {
        for word in line.split_whitespace() {
            if let Some((name, value)) = word.split_once('=') {
                env.insert(name.into(), value.trim_matches('"').into());
            }
        }
    }
    env
}

#[test]
fn standard_image_installs_and_declares_sccache() {
    let image =
        fs::read_to_string(root().join(".quecto/containers/standard/Containerfile")).unwrap();
    let instructions = image.replace("\\\n", " ");
    assert!(
        instructions
            .lines()
            .filter(|line| line.trim_start().starts_with("RUN ")
                || line.trim_start().starts_with("COPY "))
            .any(|line| line.contains("sccache")
                && ["install", "COPY", "curl", "wget"]
                    .iter()
                    .any(|action| line.contains(action))),
        "standard image does not install sccache"
    );
    let tools = image
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("LABEL ai.quecto.required-tools=\"")
        })
        .unwrap();
    assert!(
        tools
            .trim_end_matches('"')
            .split_whitespace()
            .any(|tool| tool == "sccache"),
        "standard image must ask doctor to check sccache"
    );
}

#[test]
fn standard_image_sets_wrapper_shared_cache_and_finite_size() {
    let env = image_env();
    assert!(
        env.get("RUSTC_WRAPPER")
            .is_some_and(|v| v.ends_with("sccache")),
        "standard image has no sccache wrapper"
    );
    assert!(
        env.get("SCCACHE_DIR")
            .is_some_and(|v| Path::new(v).is_absolute()),
        "standard image has no absolute SCCACHE_DIR"
    );
    assert!(
        env.get("SCCACHE_CACHE_SIZE").is_some_and(|v| {
            let digits: String = v.chars().take_while(char::is_ascii_digit).collect();
            let unit = &v[digits.len()..];
            digits.parse::<u64>().is_ok_and(|size| size > 0)
                && ["", "K", "M", "G", "T", "KB", "MB", "GB", "TB"].contains(&unit)
        }),
        "standard image has no positive finite SCCACHE_CACHE_SIZE"
    );
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "quecto-2383-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn executable(path: &Path, source: &str) {
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

struct Launch {
    _scratch: Scratch,
    base: PathBuf,
    mounts: Vec<(PathBuf, String, String)>,
    environment: BTreeSet<String>,
    matching_cache_environment: Vec<String>,
}

fn launch() -> Launch {
    let scratch = Scratch::new();
    let base = scratch.0.join("base");
    let bin = scratch.0.join("bin");
    let home = scratch.0.join("home");
    let socket = scratch.0.join("sockets");
    for path in [&base, &bin, &home.join(".quecto"), &socket] {
        fs::create_dir_all(path).unwrap();
    }
    let mounts_log = scratch.0.join("mounts");
    let names_log = scratch.0.join("names");
    // Model image defaults, followed by runtime overrides; never inherit host env.
    let defaults = image_env()
        .into_iter()
        .map(|(name, value)| {
            let value = value
                .replace("${PATH}", "/usr/bin:/bin")
                .replace("$PATH", "/usr/bin:/bin");
            format!("'{}'", format!("{name}={value}").replace('\'', "'\\''"))
        })
        .collect::<Vec<_>>()
        .join(" ");
    // No argv/environment dumps. Only mount topology and exported NAMES.
    executable(
        &bin.join("podman"),
        &format!(
            r#"#!/bin/bash
set -eu
if [ "$1" = image ]; then
  case "$2" in exists) exit 0;; inspect) printf '\n'; exit 0;; esac
fi
if [ "$1" = run ] && [ "$2" = --rm ]; then exit 0; fi
if [ "$1" = run ]; then
  shift
  runtime_env=(PATH=/usr/bin:/bin {defaults})
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -v|--volume) printf '%s\n' "$2" >> {mounts:?}; shift 2;;
      -e|--env) runtime_env+=("$2"); shift 2;;
      --mount)
        src=''; dst=''; mode=rw
        IFS=, read -ra fields <<< "$2"
        for field in "${{fields[@]}}"; do
          case "$field" in source=*|src=*) src="${{field#*=}}";; target=*|dst=*|destination=*) dst="${{field#*=}}";; readonly|ro) mode=ro;; esac
        done
        [ -n "$src" ] && [ -n "$dst" ]
        printf '%s:%s:%s\n' "$src" "$dst" "$mode" >> {mounts:?}
        shift 2;;
      --name|--hostname|--label|--user|-w|--workdir|--pids-limit) shift 2;;
      --*) shift;;
      -d) shift;;
      quecto-standard-2383:local) shift; break;;
      *) exit 125;;
    esac
  done
  env -i "${{runtime_env[@]}}" "$@"
  printf 'fixture-container\n'
  exit 0
fi
exit 125
"#,
            mounts = mounts_log
        ),
    );
    executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' fixture-not-a-credential\n",
    );
    let build = bin.join("cargo-build-presence");
    let checks_log = scratch.0.join("matches");
    fs::write(&checks_log, "").unwrap();
    let mut image_defaults = image_env();
    image_defaults
        .entry("SCCACHE_DIR".into())
        .or_insert_with(|| "/tmp/quecto-sccache".into());
    let comparisons = ["CARGO_HOME", "SCCACHE_DIR"].into_iter().map(|name| {
        let expected = image_defaults[name].replace('\'', "'\\''");
        format!("if [ \"${{{name}:-}}\" = '{expected}' ]; then printf '%s\\n' '{name}' >> {checks_log:?}; fi\n")
    }).collect::<String>();
    executable(
        &build,
        &format!(
            "#!/bin/bash\nset -euo pipefail\ncompgen -e | LC_ALL=C sort -u >> {names_log:?}\n{comparisons}"
        ),
    );
    let adapter = if let Some(project_adapter) = project_adapter() {
        project_adapter
    } else {
        eprintln!("#2383 pre-implementation baseline: shipped Docker adapter, not project opt-in");
        root().join("scripts/container-runtime/docker/create.sh")
    };
    let mut command = Command::new("timeout");
    command
        .env_clear()
        .args(["--signal=KILL", "15s", "bash"])
        .arg(adapter)
        .arg("--state-dir")
        .arg(base.join("container-environments"))
        .args(["--image", "quecto-standard-2383:local", "--"])
        .arg(build)
        .arg("--socket")
        .arg(socket.join("child.sock"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", home)
        .env("QUECTO_BASE_DIR", &base)
        .env("QUECTO_CONTAINER_CLI", "podman")
        .env("QUECTO_CONTAINER_ENVIRONMENT_REF", "standard")
        .env("QUECTO_CONTAINER_PROBE_TIMEOUT", "1");
    // Obvious fixture sentinels only; never inherit developer credentials.
    for name in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "FIREWORKS_API_KEY",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "EXAMPLE_CACHE_TEST_CREDENTIAL",
    ] {
        command.env(name, "fixture-not-a-credential");
    }
    // Two fresh environments share one base; neither launch uses a shared target.
    for _ in 0..2 {
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "controlled baseline launch must succeed before cache assertions (status {:?})",
            output.status.code()
        );
    }
    let mounts = fs::read_to_string(mounts_log)
        .unwrap()
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.rsplitn(3, ':').collect();
            assert_eq!(fields.len(), 3);
            (PathBuf::from(fields[2]), fields[1].into(), fields[0].into())
        })
        .collect();
    let environment = fs::read_to_string(names_log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let matching_cache_environment = fs::read_to_string(checks_log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    Launch {
        _scratch: scratch,
        base,
        mounts,
        environment,
        matching_cache_environment,
    }
}

fn shared_mount<'a>(run: &'a Launch, destination: &str) -> &'a Path {
    let mount = run
        .mounts
        .iter()
        .find(|(_, dst, mode)| dst == destination && mode == "rw")
        .unwrap_or_else(|| {
            panic!("standard create has no shared read-write mount for {destination}")
        });
    let matching: Vec<_> = run
        .mounts
        .iter()
        .filter(|(_, dst, mode)| dst == destination && mode == "rw")
        .collect();
    assert_eq!(
        matching.len(),
        2,
        "both fresh environments must mount the cache"
    );
    assert!(
        matching.iter().all(|other| other.0 == mount.0),
        "fresh environments must share the same cache source"
    );
    assert!(
        mount.0.starts_with(&run.base),
        "composition base must own shared cache"
    );
    assert_eq!(
        fs::metadata(&mount.0).unwrap().uid(),
        fs::metadata(&run.base).unwrap().uid(),
        "shared cache must belong to the invoking owner"
    );
    &mount.0
}

#[test]
fn standard_create_mounts_shared_owner_only_compiler_cache() {
    let run = launch();
    let destination = image_env()
        .get("SCCACHE_DIR")
        .cloned()
        .unwrap_or_else(|| "/tmp/quecto-sccache".into());
    let cache = shared_mount(&run, &destination);
    assert!(
        run.matching_cache_environment
            .iter()
            .filter(|name| name.as_str() == "SCCACHE_DIR")
            .count()
            == 2,
        "compiler environment must match its mount destination"
    );
    assert_eq!(
        fs::metadata(cache).unwrap().permissions().mode() & 0o777,
        0o700,
        "shared compiler cache must be owner-only"
    );
    assert!(
        run.environment.contains("SCCACHE_DIR"),
        "cache directory environment is absent"
    );
}

#[test]
fn standard_create_mounts_dedicated_cargo_home_with_allowed_initial_entries() {
    let run = launch();
    let destination = image_env()["CARGO_HOME"].clone();
    let cache = shared_mount(&run, &destination);
    assert!(
        run.matching_cache_environment
            .iter()
            .filter(|name| name.as_str() == "CARGO_HOME")
            .count()
            == 2,
        "Cargo environment must match its mount destination"
    );
    assert_eq!(
        fs::metadata(cache).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let entries: Vec<_> = fs::read_dir(cache)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let allowed = [
        "registry",
        "git",
        "bin",
        ".package-cache",
        ".package-cache-mutate",
        ".global-cache",
    ];
    assert!(
        entries
            .iter()
            .all(|name| allowed.iter().any(|allowed| name == allowed)),
        "dedicated Cargo home contains an entry outside its allowed initial entries"
    );
    assert!(
        run.environment.contains("CARGO_HOME"),
        "shared Cargo home environment is absent"
    );
}

#[test]
fn standard_cache_build_has_only_allowlisted_environment_names() {
    let run = launch();
    let allowed: BTreeSet<String> = [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TZ",
        "TERM",
        "TMPDIR",
        "TMP",
        "TEMP",
        "PWD",
        "OLDPWD",
        "SHLVL",
        "_",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "RUSTC_WRAPPER",
        "SCCACHE_DIR",
        "SCCACHE_CACHE_SIZE",
        "CARGO_BUILD_JOBS",
        "SCCACHE_IDLE_TIMEOUT",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let unexpected: Vec<_> = run.environment.difference(&allowed).collect();
    assert!(
        unexpected.is_empty(),
        "cache-using build exported unexpected environment NAMES: {unexpected:?}"
    );
    for required in [
        "PATH",
        "CARGO_HOME",
        "RUSTC_WRAPPER",
        "SCCACHE_DIR",
        "SCCACHE_CACHE_SIZE",
    ] {
        assert!(
            run.environment.contains(required),
            "cache-using build lacks required environment name {required}"
        );
    }
}
