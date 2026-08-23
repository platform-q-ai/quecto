use std::sync::Arc;

use crate::domain::tool::Tool;
use crate::infrastructure::security::sandbox::Sandbox;

use super::python_lab::{PythonLabConfig, PythonLabTool};

fn tool(dir: &std::path::Path) -> PythonLabTool {
    PythonLabTool::new(
        Arc::new(dir.to_path_buf()),
        Arc::new(Sandbox::new(Some(dir.to_path_buf()))),
        PythonLabConfig {
            default_timeout_seconds: 1,
            max_foreground_seconds: 2,
            default_max_output_bytes: 8,
            max_output_bytes: 32,
            ..Default::default()
        },
    )
}

#[tokio::test]
async fn reports_actual_interpreter_version() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let result = lab
        .execute(r#"{"op":"run","code":"1","background":true}"#)
        .await
        .unwrap();
    let started: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let job_id = started["job_id"].as_str().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let status = lab
        .execute(&format!(r#"{{"op":"status","job_id":"{job_id}"}}"#))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&status.content).unwrap();
    let reported = v["interpreter_version"].as_str().unwrap();
    assert_ne!(reported, "python3");
    assert!(reported.starts_with("Python "), "{reported}");
}

#[cfg(unix)]
fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and only reads the current effective uid.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    // SAFETY: kill(pid, 0) performs existence/permission checking without sending a signal.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(unix)]
#[tokio::test]
async fn process_limit_enforces_attempted_subprocess_creation() {
    if running_as_root() {
        eprintln!("skipping RLIMIT_NPROC enforcement test for root");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let code = r#"import os,subprocess,sys
blocked=[]
for f in (lambda: os.fork(), lambda: subprocess.Popen([sys.executable,'-c','1'])):
    try:
        p=f()
        if isinstance(p,int): os._exit(0) if p==0 else os.waitpid(p,0)
        else: p.wait()
        blocked.append('allowed')
    except OSError: pass
sys.exit(1 if blocked else 0)
"#;
    let result = tool(tmp.path())
        .execute(&serde_json::json!({"op":"run","code":code,"timeout_seconds":2}).to_string())
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "process creation was not blocked: {}",
        result.content
    );
}

#[cfg(unix)]
#[tokio::test]
async fn drop_terminates_background_job_process() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = PythonLabTool::new(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()))),
        PythonLabConfig {
            default_timeout_seconds: 30,
            max_processes: None,
            ..Default::default()
        },
    );
    let started = lab.execute(r#"{"op":"run","code":"import os,pathlib,time; pathlib.Path('pid.txt').write_text(str(os.getpid())); time.sleep(30)","background":true}"#).await.unwrap();
    assert!(!started.is_error, "{}", started.content);
    for _ in 0..50 {
        if tmp.path().join("pid.txt").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let pid: u32 = std::fs::read_to_string(tmp.path().join("pid.txt"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(pid_is_alive(pid));
    drop(lab);
    for _ in 0..50 {
        if !pid_is_alive(pid) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("background Python process {pid} survived PythonLabTool drop");
}

#[cfg(unix)]
#[tokio::test]
async fn memory_cpu_and_process_rlimits_are_enforced() {
    if running_as_root() {
        eprintln!("skipping RLIMIT_NPROC portion for root");
    }
    let tmp = tempfile::tempdir().unwrap();
    let mem_lab = PythonLabTool::new(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()))),
        PythonLabConfig {
            max_memory_bytes: Some(64 * 1024 * 1024),
            default_timeout_seconds: 5,
            max_processes: None,
            ..Default::default()
        },
    );
    let mem = mem_lab
        .execute(r#"{"op":"run","code":"x=bytearray(512*1024*1024)","timeout_seconds":5}"#)
        .await
        .unwrap();
    assert!(
        mem.is_error,
        "memory limit was not enforced: {}",
        mem.content
    );

    let cpu_lab = PythonLabTool::new(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()))),
        PythonLabConfig {
            max_cpu_seconds: Some(1),
            default_timeout_seconds: 5,
            max_processes: None,
            ..Default::default()
        },
    );
    let cpu = cpu_lab
        .execute(r#"{"op":"run","code":"while True: pass","timeout_seconds":5}"#)
        .await
        .unwrap();
    let cpu_v: serde_json::Value = serde_json::from_str(&cpu.content).unwrap();
    // Deliberately does not accept `cpu.is_error` on its own: the 5s wall-clock
    // timeout also sets that, so the disjunct would pass with RLIMIT_CPU never
    // applied. SIGXCPU must kill the interpreter well before the timeout.
    assert_eq!(
        cpu_v["status"], "completed",
        "CPU limit should end the run before the wall-clock timeout: {}",
        cpu.content
    );
    assert!(
        cpu_v["duration_ms"].as_u64().unwrap_or(u64::MAX) < 4_500,
        "CPU limit was not enforced before timeout: {}",
        cpu.content
    );

    if !running_as_root() {
        let proc_result = tool(tmp.path()).execute(r#"{"op":"run","code":"import os; pid=os.fork(); os._exit(0) if pid == 0 else os.waitpid(pid, 0)","timeout_seconds":2}"#).await.unwrap();
        assert!(
            proc_result.is_error,
            "process limit was not enforced: {}",
            proc_result.content
        );
    }
}
