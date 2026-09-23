//! SPIKE: replay Agent Commander dry-run logs through the current questions.
//! Usage: commander_replay <out_dir> <since_epoch_secs> <log.jsonl>...
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() >= 4,
        "usage: commander_replay <out_dir> <since> <log.jsonl>..."
    );
    let since: f64 = args[2].parse().expect("since is epoch seconds");
    let out = Path::new(&args[1]);
    for file in &args[3..] {
        let text = std::fs::read_to_string(file).expect("readable log");
        let records: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).expect("jsonl record"))
            .filter(|r: &serde_json::Value| {
                r["ts"]
                    .as_str()
                    .and_then(|t| t.parse::<f64>().ok())
                    .is_some_and(|t| t >= since)
            })
            .collect();
        if records.is_empty() {
            continue;
        }
        match quecto::infrastructure::agent_commander::replay_session(&records, out) {
            Ok(n) => eprintln!("{file}: {n} replayed"),
            Err(e) => eprintln!("{file}: FAILED {e}"),
        }
    }
}
