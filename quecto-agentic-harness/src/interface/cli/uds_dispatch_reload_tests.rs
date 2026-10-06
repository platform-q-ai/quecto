//! The rebuild phase must not stall the current-thread UDS runtime (#2022
//! review F1): while a slow source rebuilds, sibling tasks on the same
//! runtime keep progressing.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::application::catalogue::ports::{ReloadedConfiguration, RuntimeConfigurationSource};

const REBUILD_TAKES: Duration = Duration::from_millis(200);

/// A source whose rebuild sleeps synchronously, as a real config read plus
/// provider composition does.
#[derive(Debug)]
struct SlowSource;

impl RuntimeConfigurationSource for SlowSource {
    fn changed(&mut self) -> bool {
        true
    }
    fn rebuild(&mut self) -> Result<ReloadedConfiguration, String> {
        std::thread::sleep(REBUILD_TAKES);
        Err("slow source never composes".to_string())
    }
}

/// A source whose rebuild panics, as a composition bug would.
#[derive(Debug)]
struct PanickingSource;

impl RuntimeConfigurationSource for PanickingSource {
    fn changed(&mut self) -> bool {
        true
    }
    fn rebuild(&mut self) -> Result<ReloadedConfiguration, String> {
        panic!("composition bug")
    }
}

fn current_thread_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("current-thread runtime")
}

/// Ticks a counter every millisecond until told to stop, standing in for
/// the accept loop, client readers and broadcast writer.
fn spawn_ticker(ticks: Arc<AtomicUsize>, stop: Arc<std::sync::atomic::AtomicBool>) {
    tokio::spawn(async move {
        while !stop.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
            ticks.fetch_add(1, Ordering::SeqCst);
        }
    });
}

#[test]
fn a_slow_rebuild_lets_sibling_tasks_on_the_current_thread_runtime_progress() {
    for forced in [true, false] {
        let ticks = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reload = Arc::new(ReloadRuntimeConfiguration::new(Box::new(SlowSource)));
        let step = current_thread_runtime().block_on(async {
            spawn_ticker(ticks.clone(), stop.clone());
            let step = rebuild_off_runtime(reload, forced).await;
            stop.store(true, Ordering::SeqCst);
            step
        });
        let ticked = ticks.load(Ordering::SeqCst);
        assert!(
            ticked > 0,
            "forced={forced}: the ticker never ran while the {REBUILD_TAKES:?} rebuild was in flight — the rebuild blocked the runtime"
        );
        if forced {
            assert!(
                matches!(&step, ReloadStep::Failed(error) if error == "slow source never composes")
            );
        } else {
            assert!(matches!(step, ReloadStep::Unchanged));
        }
    }
}

#[test]
fn a_panicking_rebuild_worker_reports_failure_when_forced_and_retains_last_good_when_polled() {
    let reload = Arc::new(ReloadRuntimeConfiguration::new(Box::new(PanickingSource)));
    let (forced, polled) = current_thread_runtime().block_on(async {
        (
            rebuild_off_runtime(reload.clone(), true).await,
            rebuild_off_runtime(reload, false).await,
        )
    });
    assert!(matches!(&forced, ReloadStep::Failed(error) if error == WORKER_PANICKED));
    assert!(matches!(polled, ReloadStep::Unchanged));
}

/// Only `list_models` polls before the dispatch loop answers it (#2451).
#[test]
fn only_a_listing_polls_before_its_reply() {
    let command = |json: &str| {
        serde_json::from_str::<crate::interface::cli::protocol::AgentCommand>(json)
            .expect("a protocol command")
    };
    assert!(polls_before_reply(&command(
        r#"{"type":"list_models","id":"lm"}"#
    )));
    for other in [
        r#"{"type":"prompt","id":"p","message":"hi"}"#,
        r#"{"type":"set_model","id":"sm","model":"openai-api/gpt-6-sol"}"#,
        r#"{"type":"get_state","id":"gs"}"#,
        r#"{"type":"refresh_models","id":"rm"}"#,
        r#"{"type":"get_tool_catalogue","id":"tc"}"#,
    ] {
        assert!(!polls_before_reply(&command(other)), "{other}");
    }
}
