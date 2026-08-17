use super::*;
use crate::shell::cli::{build_agent_args, parse_flags};

#[test]
fn tab_spawn_flags_inherit_parent_policy() {
    let parent = parse_flags(&[
        "quecto-tui".into(),
        "--workflow".into(),
        "--workflow-guards".into(),
        "--config".into(),
        "/tmp/q.toml".into(),
        "--system".into(),
        "be brief".into(),
        "--disable-tool".into(),
        "bash".into(),
    ]);
    let policy = TabSpawnPolicy::from_flags(&parent);
    let flags = tab_spawn_flags_from_policy(&policy, Some("sess".into()));
    assert!(flags.workflow, "F8: inherit --workflow");
    assert!(flags.workflow_guards, "F8: inherit --workflow-guards");
    assert_eq!(
        flags.config_path.as_deref(),
        Some(std::path::Path::new("/tmp/q.toml"))
    );
    assert_eq!(flags.system_prompt.as_deref(), Some("be brief"));
    assert_eq!(flags.disable_tools, vec!["bash".to_string()]);
    assert!(flags.persist, "secondary tabs keep persist default");
    let built = build_agent_args(&flags);
    assert!(
        !built.iter().any(|a| a == "sess" || a.contains("resume")),
        "F5: resume must not be dropped into CLI args: {built:?}"
    );
}

#[test]
fn tab_spawn_flags_default_persist_without_parent() {
    let flags = tab_spawn_flags(None);
    assert!(flags.persist);
}
