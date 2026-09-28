use std::time::Duration;

use super::OwnedChildSupervisor;

#[test]
fn a_pump_outlives_the_runtime_that_spawned_it() {
    let supervisor = OwnedChildSupervisor::new();
    let (to_pump, mut from_test) = tokio::sync::mpsc::channel::<u32>(1);
    let (to_test, mut from_pump) = tokio::sync::mpsc::channel::<u32>(1);
    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // Spawned from inside the caller's runtime, as a launcher would be.
    let entered = caller.enter();
    let pump = supervisor.spawn_pump(async move {
        while let Some(n) = from_test.recv().await {
            if to_test.send(n + 1).await.is_err() {
                return;
            }
        }
    });
    drop(entered);
    drop(caller);
    let reader = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    reader.block_on(async {
        to_pump.send(41).await.unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(10), from_pump.recv())
            .await
            .expect("the pump answers");
        assert_eq!(answer, Some(42));
        drop(to_pump);
        tokio::time::timeout(Duration::from_secs(10), pump)
            .await
            .expect("the pump ends with its input")
            .unwrap();
    });
}

#[test]
fn a_pump_is_spawned_from_outside_any_runtime() {
    let supervisor = OwnedChildSupervisor::new();
    let pump = supervisor.spawn_pump(async { 7 });
    let reader = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(reader.block_on(pump).unwrap(), 7);
}
