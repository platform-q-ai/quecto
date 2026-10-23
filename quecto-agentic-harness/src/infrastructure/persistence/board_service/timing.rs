//! SPIKE (board service): where a `quecto board` call's time goes, printed
//! to standard error at exit while `QUECTO_BOARD_TIMING` is set: process
//! start to `main`, composition, git spawns, HTTP calls and the server's
//! own time.
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Tally {
    started: Option<Instant>,
    marks: Vec<(String, Duration)>,
    git: Vec<(String, Duration)>,
    http: Vec<(String, Duration, u64)>,
}

static TALLY: Mutex<Option<Tally>> = Mutex::new(None);

fn with(body: impl FnOnce(&mut Tally)) {
    let mut tally = TALLY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    body(tally.get_or_insert_with(Tally::default));
}

pub fn enabled() -> bool {
    std::env::var_os("QUECTO_BOARD_TIMING").is_some()
}

/// `main` began.
pub fn begin() {
    with(|tally| tally.started = Some(Instant::now()));
}

pub fn mark(name: &str) {
    with(|tally| {
        let at = tally
            .started
            .map(|started| started.elapsed())
            .unwrap_or_default();
        tally.marks.push((name.to_string(), at));
    });
}

pub fn git(args: &[&str], took: Duration) {
    with(|tally| {
        tally.git.push((
            args.iter().take(2).copied().collect::<Vec<_>>().join(" "),
            took,
        ))
    });
}

/// One HTTP call: its path, its time, and the server's own time (µs).
pub fn http(path: &str, took: Duration, server_micros: u64) {
    with(|tally| tally.http.push((path.to_string(), took, server_micros)));
}

/// The process's age at `main`: from its start time in `/proc/self/stat`.
fn process_age_ms() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after = stat.rsplit_once(')')?.1;
    let start_ticks: f64 = after.split_whitespace().nth(19)?.parse().ok()?;
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let ticks = 100.0;
    Some((uptime - start_ticks / ticks) * 1000.0)
}

/// Prints the tally, as one JSON line on standard error.
pub fn report() {
    if !enabled() {
        return;
    }
    with(|tally| {
        let total = tally
            .started
            .map(|started| started.elapsed())
            .unwrap_or_default();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let git_ms: f64 = tally.git.iter().map(|(_, d)| ms(*d)).sum();
        let http_ms: f64 = tally.http.iter().map(|(_, d, _)| ms(*d)).sum();
        let server_ms: f64 = tally
            .http
            .iter()
            .map(|(_, _, us)| *us as f64 / 1000.0)
            .sum();
        let line = serde_json::json!({
            "age_at_exit_ms": process_age_ms(),
            "main_to_exit_ms": ms(total),
            "marks": tally.marks.iter().map(|(n, d)| serde_json::json!([n, ms(*d)])).collect::<Vec<_>>(),
            "git_spawns": tally.git.len(),
            "git_ms": git_ms,
            "git": tally.git.iter().map(|(a, d)| serde_json::json!([a, ms(*d)])).collect::<Vec<_>>(),
            "http_calls": tally.http.len(),
            "http_ms": http_ms,
            "server_ms": server_ms,
        });
        eprintln!("QUECTO_BOARD_TIMING {line}");
    });
}
