//! Disposable fixed-pool observation runner; not a new lifecycle service.
use quecto::{
    domain::tool::Tool,
    infrastructure::{
        config::Config,
        security::sandbox::Sandbox,
        tools::{
            python_lab::PythonLabTool,
            swarm::{SwarmTool, require_container},
        },
    },
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

async fn python(tool: &SwarmTool, code: String) -> Result<String, String> {
    let result = tool
        .execute(&json!({"op":"run","code":code}).to_string())
        .await
        .map_err(|e| e.to_string())?;
    if result.is_error {
        Err(result.content)
    } else {
        Ok(result.content)
    }
}
fn quote(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    require_container()?;
    if quecto::infrastructure::tools::swarm::enabled() {
        return Err("fixed swarm workers cannot start another runner; reuse existing pool".into());
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        return Err("usage: quecto-swarm-spike demo|inspect|cancel|live DB [CONFIG MEMBERS SECONDS]; live requires explicit config; MEMBERS=2..10 includes scripted coordinator; Ctrl-C cancels".into());
    }
    let path = std::path::absolute(&args[1]).map_err(|e| e.to_string())?;
    let config = if let Some(config) = args.get(2) {
        Config::load(config).map_err(|e| e.to_string())?
    } else {
        Config::default()
    };
    let workspace = std::env::current_dir().map_err(|e| e.to_string())?;
    let tool = SwarmTool::new(PythonLabTool::new(
        Arc::new(workspace.clone()),
        Arc::new(Sandbox::new(Some(workspace))),
        config.tools.python_lab.into(),
    ));
    let db = quote(&path);
    match args[0].as_str() {
        "demo" => println!("{}", python(&tool, format!("import json; print(json.dumps(swarm.demo({db}), indent=2))")).await?),
        "inspect" => println!("{}", python(&tool, format!("import json; print(json.dumps(swarm.Swarm({db}).summary(), indent=2))")).await?),
        "cancel" => println!("{}", python(&tool, format!("s=swarm.Swarm({db}); s.cancel('coordinator', 'user cancellation'); print(s.summary())")).await?),
        "live" => {
            let config = args.get(2).ok_or("live requires CONFIG MEMBERS SECONDS")?;
            let count: usize = args.get(3).ok_or("MEMBERS required")?.parse().map_err(|_| "invalid MEMBERS")?;
            let seconds: u64 = args.get(4).ok_or("SECONDS required")?.parse().map_err(|_| "invalid SECONDS")?;
            if !(2..=10).contains(&count) || !(5..=600).contains(&seconds) { return Err("MEMBERS must be 2..10 including coordinator; SECONDS 5..600".into()); }
            live(&tool, &path, config, count, seconds).await?;
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
async fn live(
    tool: &SwarmTool,
    path: &Path,
    config: &str,
    count: usize,
    seconds: u64,
) -> Result<(), String> {
    // One live spike pool per Unix user/container. Keep the advisory lock until
    // all children are reaped; never unlink a lock inode while another opener waits.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(Path::new("/tmp/quecto-swarm-spike-pool.lock"))
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(
                "a swarm spike pool is already reserved in this container; reuse it".into(),
            );
        }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let db = quote(path);
    let members: Vec<_> = std::iter::once("coordinator".to_string())
        .chain((1..count).map(|i| format!("worker-{i}")))
        .collect();
    python(tool, format!("import time\ns=swarm.Swarm.create({db},goal='Compute squares 1..5 and exchange results',done='Coordinator independently checks five square evidence records and total 55',deadline=time.time()+{seconds},coordinator='coordinator',members={},token_budget=0)\nfor n in range(1,6): s.add_task('coordinator',str(n),f'Compute square of {{n}}; evidence must be a dict with integer n and square')\nfor m in {}: s.message('coordinator',m,'Claim ready tasks, publish evidence and send results to coordinator')", serde_json::to_string(&members).unwrap(), serde_json::to_string(&members[1..]).unwrap())).await?;
    let executable: PathBuf = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("quecto");
    let logs = path.with_extension("logs");
    std::fs::create_dir(&logs).map_err(|e| e.to_string())?;
    let outputs = members[1..]
        .iter()
        .map(|member| {
            Ok((
                std::fs::File::create(logs.join(format!("{member}.stdout")))?,
                std::fs::File::create(logs.join(format!("{member}.stderr")))?,
            ))
        })
        .collect::<Result<Vec<_>, std::io::Error>>()
        .map_err(|e| e.to_string())?;
    let mut children = Vec::new();
    let mut launch_error = None;
    let mut process_groups = Vec::new();
    for (member, (stdout, stderr)) in members[1..].iter().zip(outputs) {
        let prompt = format!(
            "You are {member}, fixed swarm worker. Safe arithmetic only; no spawning, shell, file edits or Git. Use swarm tool code with import swarm; s=swarm.Swarm({db}). Inspect s.summary() and s.messages('{member}'). Repeatedly c=s.claim('{member}'); if c is None stop (never busy poll). Task id is c['task_id']; inspect c if needed. For task n compute n*n, s.finish('{member}', task_id, c['claim_token'], evidence=[{{'n':n,'square':n*n}}], tokens_used=0); s.message('{member}','coordinator','task '+task_id+' result '+str(n*n)). Take more tasks in the same Python call or subsequent calls. Do not complete overall run. End with short summary. Helpers are cooperative. No live provider token cap is claimed."
        );
        let mut command = tokio::process::Command::new(&executable);
        command
            .args([
                "--config",
                config,
                "agent",
                "--no-session",
                "--max-time",
                &seconds.to_string(),
                "--max-iterations",
                "12",
                "--disable-tool",
                "spawn",
                "--disable-tool",
                "agent_cmd",
                "--disable-tool",
                "bash",
                "--message",
                &prompt,
            ])
            .env("QUECTO_SWARM_SPIKE", "1")
            .stdout(stdout)
            .stderr(stderr)
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        match command.spawn() {
            Ok(child) => {
                if let Some(pid) = child.id() {
                    process_groups.push(pid);
                }
                children.push(child);
            }
            Err(e) => {
                launch_error = Some(e.to_string());
                break;
            }
        }
    }
    println!(
        "DB={} logs={} pool={count} (one scripted coordinator); Ctrl-C cancels",
        path.display(),
        logs.display()
    );
    let cancelled = if launch_error.is_some() {
        true
    } else {
        let work = async {
            for child in &mut children {
                if !child.wait().await.map_err(|e| e.to_string())?.success() {
                    return Err("worker failed; see logs".to_string());
                }
            }
            Ok::<(), String>(())
        };
        tokio::select! {
            result = work => { if let Err(e) = result { launch_error=Some(e); true } else { false } },
            _ = tokio::signal::ctrl_c() => true,
            _ = tokio::time::sleep_until(deadline) => true,
        }
    };
    if cancelled {
        // Mark the board first, then terminate and reap every process before exit.
        let settlement = tokio::time::timeout(Duration::from_secs(2), python(tool, format!("s=swarm.Swarm({db}); s.cancel('coordinator','runner cancellation, deadline or launch/worker failure'); print(s.summary())"))).await;
        #[cfg(unix)]
        for pid in &process_groups {
            // Groups are captured before wait clears Child::id(). No replacements.
            unsafe {
                libc::kill(-(*pid as i32), libc::SIGKILL);
            }
        }
        for child in &mut children {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        let settlement = settlement.map_err(|_| {
            "board cancellation timed out; workers killed, inspect partial DB".to_string()
        })?;
        println!("{}", settlement?);
        return Err(launch_error
            .unwrap_or_else(|| "cancelled or deadline exhausted; partial DB preserved".into()));
    }
    println!("{}", python(tool, format!("s=swarm.Swarm({db})\nsummary=s.summary()\nprint(summary)\nimport json\nrows=[e for r in summary['evidence'] for e in json.loads(r['payload'])]\nassert sorted((e['n'],e['square']) for e in rows)==[(n,n*n) for n in range(1,6)], 'incomplete or invalid evidence'\ns.complete('coordinator','Independently verified five squares and sum 55')\nprint(s.summary())")).await?);
    Ok(())
}
