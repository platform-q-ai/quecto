use cucumber::given;

use super::QuectoWorld;

/// Enable workflow in the existing config file.
#[given("workflow is enabled in config")]
fn given_workflow_enabled(world: &mut QuectoWorld) {
    set_workflow_enabled(world, true);
}

/// Disable workflow in the existing config file.
#[given("workflow is disabled in config")]
fn given_workflow_disabled(world: &mut QuectoWorld) {
    set_workflow_enabled(world, false);
}

fn set_workflow_enabled(world: &mut QuectoWorld, enabled: bool) {
    super::ensure_temp_dir(world);
    let base = super::base_path(world);
    let config_path = base.join("config.json");
    let mut config: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path).unwrap();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        serde_json::json!({})
    };
    config["workflow"]["enabled"] = serde_json::json!(enabled);
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
}
