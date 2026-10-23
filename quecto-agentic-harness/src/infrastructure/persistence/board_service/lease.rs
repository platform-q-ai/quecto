//! SPIKE (board service): the primary's lease, a ref on a GitHub
//! repository moved only by compare-and-swap (`git push
//! --force-with-lease=<ref>:<expected>`). The holder renews it every few
//! seconds with a new root commit naming itself and its epoch; it takes
//! writes only until `renewal start + ttl - margin`. A standby watches the
//! ref with `ls-remote` alone: once it has seen no change for `ttl` (on
//! its own monotonic clock, counted from when it saw the last change, which
//! is after the holder began that renewal), the lease has lapsed, and it
//! may take it by a compare-and-swap from the very id it saw. No normal
//! board read or write touches the ref.
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Clone)]
pub struct LeaseConfig {
    /// The repository holding the lease ref (a scratch repository).
    pub url: String,
    /// `refs/quecto/board-lease/<board-id>`.
    pub reference: String,
    pub holder: String,
    pub ttl: Duration,
    /// How often the holder renews, and a standby polls.
    pub every: Duration,
    /// A local scratch git directory to build lease commits in.
    pub git_dir: PathBuf,
}

/// Where the holder's lease stands.
#[derive(Debug, Default)]
pub struct LeaseState {
    pub valid_until: Option<Instant>,
    pub last_oid: Option<String>,
    pub fenced: bool,
    pub renewals: u64,
    pub failures: u64,
    pub last_renew_ms: f64,
}

pub type SharedLease = Arc<Mutex<LeaseState>>;

fn git(config: &LeaseConfig, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .arg("--git-dir")
        .arg(&config.git_dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", &config.holder)
        .env("GIT_AUTHOR_EMAIL", "lease@quecto.invalid")
        .env("GIT_COMMITTER_NAME", &config.holder)
        .env("GIT_COMMITTER_EMAIL", "lease@quecto.invalid")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("git: {error}"))
}

/// Makes the scratch git directory, with the empty tree in it.
///
/// # Errors
/// git's.
pub fn prepare(config: &LeaseConfig) -> Result<(), String> {
    if !config.git_dir.join("HEAD").exists() {
        let out = Command::new("git")
            .args(["init", "--bare", "-q"])
            .arg(&config.git_dir)
            .output()
            .map_err(|error| error.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
    }
    let out = git(config, &["hash-object", "-t", "tree", "-w", "/dev/null"])?;
    let tree = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match tree == EMPTY_TREE {
        true => Ok(()),
        false => Err(format!("unexpected empty tree {tree}")),
    }
}

/// The ref's id on the remote, or `None` when it is absent.
///
/// # Errors
/// The remote could not be read.
pub fn observe(config: &LeaseConfig) -> Result<Option<String>, String> {
    let out = git(config, &["ls-remote", &config.url, &config.reference])?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.split_once('\t'))
        .filter(|(_, name)| *name == config.reference)
        .map(|(oid, _)| oid.to_string()))
}

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn lease_commit(config: &LeaseConfig, epoch: u64) -> Result<String, String> {
    let n = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let now = humantime::format_rfc3339_millis(std::time::SystemTime::now());
    let message = format!("holder={} epoch={epoch} n={n} at={now}", config.holder);
    let out = git(config, &["commit-tree", EMPTY_TREE, "-m", &message])?;
    match out.status.success() {
        true => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        false => Err(String::from_utf8_lossy(&out.stderr).into_owned()),
    }
}

/// Moves the ref to a new lease commit only if it is still at `expected`
/// (absent when `None`): `Ok(Some(new id))` when it moved, `Ok(None)`
/// when someone else holds it now.
///
/// # Errors
/// The push failed for any other reason (the remote out of reach).
pub fn swap(
    config: &LeaseConfig,
    expected: Option<&str>,
    epoch: u64,
) -> Result<Option<String>, String> {
    let new = lease_commit(config, epoch)?;
    let lease = format!(
        "--force-with-lease={}:{}",
        config.reference,
        expected.unwrap_or("")
    );
    let spec = format!("{new}:{}", config.reference);
    let out = git(
        config,
        &[
            "push",
            "--porcelain",
            "--no-verify",
            &lease,
            &config.url,
            &spec,
        ],
    )?;
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    match (
        out.status.success(),
        said.contains("[rejected]") || said.contains("stale info"),
    ) {
        (true, _) => Ok(Some(new)),
        (false, true) => Ok(None),
        (false, false) => Err(said.trim().to_string()),
    }
}

/// Renews the held lease every `every` until fenced or `stop`: each
/// renewal is valid from its start for `ttl - margin`.
pub fn renew_forever(config: LeaseConfig, state: SharedLease, epoch: u64, stop: Arc<AtomicBool>) {
    let margin = config.ttl / 4;
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let started = Instant::now();
            let expected = state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last_oid
                .clone();
            let swapped = swap(&config, expected.as_deref(), epoch);
            let took = started.elapsed();
            {
                let mut lease = state.lock().unwrap_or_else(|e| e.into_inner());
                lease.last_renew_ms = took.as_secs_f64() * 1000.0;
                match swapped {
                    Ok(Some(oid)) => {
                        lease.last_oid = Some(oid);
                        lease.valid_until = Some(started + config.ttl - margin);
                        lease.renewals += 1;
                    }
                    Ok(None) => {
                        eprintln!("lease: {} lost the lease: fenced", config.holder);
                        lease.fenced = true;
                        lease.valid_until = None;
                        return;
                    }
                    Err(why) => {
                        lease.failures += 1;
                        eprintln!("lease: renewal failed ({why}); valid until it lapses");
                    }
                }
            }
            std::thread::sleep(config.every.saturating_sub(started.elapsed()));
        }
    });
}

/// Whether the holder may write now.
pub fn writable(state: &SharedLease) -> Result<(), String> {
    let lease = state.lock().unwrap_or_else(|e| e.into_inner());
    match (lease.fenced, lease.valid_until) {
        (false, Some(until)) if Instant::now() < until => Ok(()),
        (true, _) => Err("this server lost the board lease: fenced".into()),
        (false, _) => Err("this server's board lease lapsed: it takes no writes".into()),
    }
}

/// Watches the ref until it has not changed for `ttl`: then answers the
/// id last seen (`None`: absent), which a take swaps from. Returns `None`
/// overall when `stop` is set first.
pub fn await_lapse(config: &LeaseConfig, stop: &AtomicBool) -> Option<Option<String>> {
    let mut seen: Option<Option<String>> = None;
    let mut seen_at = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        match observe(config) {
            Ok(oid) if seen.as_ref() != Some(&oid) => {
                seen = Some(oid);
                seen_at = Instant::now();
            }
            Ok(_) => {}
            Err(why) => eprintln!("lease: cannot read the lease ({why})"),
        }
        match &seen {
            Some(None) => return Some(None),
            Some(oid) if seen_at.elapsed() > config.ttl => return Some(oid.clone()),
            _ => {}
        }
        std::thread::sleep(config.every);
    }
    None
}
