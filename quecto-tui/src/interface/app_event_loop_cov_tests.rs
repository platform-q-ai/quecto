use super::tui_harness::TuiHarness;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn files_autocomplete_lazy_load_request_is_spawned_once_and_applied() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("open @");
    a.refresh_files_autocomplete_from_editor();
    assert!(a.files_autocomplete.take_load_request());

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let mut in_flight = false;
    a.start_files_autocomplete_load(&tx, &mut in_flight);
    assert!(in_flight, "first request should start a background load");
    a.start_files_autocomplete_load(&tx, &mut in_flight);
    assert!(in_flight, "second request while in-flight must be a no-op");
    drop(tx);

    let files = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("file load worker should respond")
        .expect("worker should send a file list");
    a.files_autocomplete.apply_loaded_files(files);
    assert!(
        in_flight,
        "worker completion normally clears this flag in the event loop"
    );
    a.refresh_files_autocomplete_from_editor();
    assert!(!a.files_autocomplete.take_load_request());
}
