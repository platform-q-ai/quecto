use crate::infrastructure::extensions::native::{
    AgentControlToolDeps, SessionToolDeps, build_agent_control_tool_extensions,
    build_session_tool_extensions, register_bundled_native_tools,
};
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

#[test]
fn bundled_native_registration_preserves_provider_ids() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut registry = ToolRegistryImpl::new();
    register_bundled_native_tools(
        &mut registry,
        build_session_tool_extensions(SessionToolDeps {
            spill_store: std::sync::Arc::new(FileContextSpillStore::new(tmp.path().to_path_buf())),
            session_key: "provider-test".to_string(),
        }),
    );
    register_bundled_native_tools(
        &mut registry,
        build_agent_control_tool_extensions(AgentControlToolDeps {
            parent_config_path: None,
            base_dir: tmp.path().to_path_buf(),
            socket_dir: tmp.path().to_path_buf(),
            restrict_to_workspace: true,
            broadcast_tx: None,
            parent_session_name: None,
            inherited_tool_policy: None,
        })
        .extensions,
    );

    let entries = registry.catalogue_entries();
    let recall = entries.iter().find(|entry| entry.name == "recall").unwrap();
    assert_eq!(recall.provider_id, "quecto:session-tools");
    assert_eq!(recall.owner, "quecto:official-tools");

    let spawn = entries.iter().find(|entry| entry.name == "spawn").unwrap();
    assert_eq!(spawn.provider_id, "quecto:agent-control");
    assert_eq!(spawn.owner, "quecto:official-tools");
}
