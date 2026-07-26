use super::tui_harness::TuiHarness;
use crate::interface::component::Component;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn files_autocomplete_lazy_load_request_is_spawned_once_and_applied() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("open @");
    a.refresh_files_autocomplete_from_editor();
    assert!(a.workspace.files_autocomplete.take_load_request());

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
    a.workspace.files_autocomplete.apply_loaded_files(files);
    assert!(
        in_flight,
        "worker completion normally clears this flag in the event loop"
    );
    a.refresh_files_autocomplete_from_editor();
    assert!(!a.workspace.files_autocomplete.take_load_request());
}

#[tokio::test]
async fn app_workspace_file_autocomplete_uses_production_visible_capacity() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("open @file");
    a.refresh_files_autocomplete_from_editor();
    assert!(a.workspace.files_autocomplete.take_load_request());

    a.workspace.files_autocomplete.apply_loaded_files(
        (0..20)
            .map(|i| format!("file-{i:02}.rs"))
            .collect::<Vec<_>>(),
    );
    a.refresh_files_autocomplete_from_editor();

    let rendered = a.workspace.files_autocomplete.render(80);
    let rows = rendered.join("\n");
    assert_eq!(
        rendered.len(),
        9,
        "8 visible file rows plus overflow indicator"
    );
    assert!(rows.contains("file-00.rs"));
    assert!(rows.contains("file-07.rs"));
    assert!(rows.contains("(1/20)"));
}

#[tokio::test]
async fn files_autocomplete_loaded_files_are_accepted_by_tab_completion() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("open @fi");
    a.refresh_files_autocomplete_from_editor();
    assert!(a.workspace.files_autocomplete.take_load_request());

    a.workspace
        .files_autocomplete
        .apply_loaded_files(vec!["first.rs".into(), "src/other.rs".into()]);
    a.refresh_files_autocomplete_from_editor();
    a.handle_key(crate::interface::keys::Key::Tab);

    assert_eq!(a.editor.text(), "open @first.rs ");
}
