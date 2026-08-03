use super::tui_harness::TuiHarness;
use crate::components::component::Component;

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

    let (root, files) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("file load worker should respond")
        .expect("worker should send a file list");
    assert!(a.apply_files_autocomplete_load(root, files));
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
    a.handle_key(crate::shell::keys::Key::Tab);

    assert_eq!(a.editor.text(), "open @first.rs ");
}

#[tokio::test]
async fn stale_files_autocomplete_load_from_previous_workspace_is_discarded() {
    let mut h = harness().await;
    let a = h.app_mut();
    let old_root = tempfile::tempdir().expect("old workspace tempdir");
    let new_root = tempfile::tempdir().expect("new workspace tempdir");
    a.workspace.root = Some(new_root.path().to_path_buf());
    a.workspace
        .files_autocomplete
        .apply_loaded_files(vec!["new.rs".into()]);

    let applied = a.apply_files_autocomplete_load(
        old_root.path().to_path_buf(),
        vec!["stale-from-old-workspace.rs".into()],
    );

    assert!(!applied, "stale worker result must not be applied");
    a.editor.set_text("open @stale");
    a.refresh_files_autocomplete_from_editor();
    assert!(
        a.workspace.files_autocomplete.render(80).is_empty(),
        "old workspace files must not appear after switching roots"
    );
}

#[tokio::test]
async fn files_autocomplete_load_for_current_workspace_is_applied() {
    let mut h = harness().await;
    let a = h.app_mut();
    let root = tempfile::tempdir().expect("workspace tempdir");
    a.workspace.root = Some(root.path().to_path_buf());

    let applied =
        a.apply_files_autocomplete_load(root.path().to_path_buf(), vec!["current.rs".into()]);

    assert!(applied, "current workspace worker result should be applied");
    a.editor.set_text("open @cur");
    a.refresh_files_autocomplete_from_editor();
    let rendered = a.workspace.files_autocomplete.render(80).join("\n");
    assert!(rendered.contains("current.rs"));
}
