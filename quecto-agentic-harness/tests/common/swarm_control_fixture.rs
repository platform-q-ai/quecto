use quecto::infrastructure::tools::swarm_bridge::SwarmContext;

pub fn context() -> (tempfile::TempDir, SwarmContext) {
    let directory = tempfile::tempdir().unwrap();
    let context = SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
    };
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    context.create_run(&serde_json::json!({"goal":"contract", "constraints":[], "criteria":[{"id":"tests","kind":"command","description":"pass"}], "member_limit":2, "deadline":deadline}),
        &quecto::domain::swarm::ProcessIdentity { pid: std::process::id(), started: quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap() }, None).unwrap();
    (directory, context)
}
