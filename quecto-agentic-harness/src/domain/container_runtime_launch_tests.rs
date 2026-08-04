use super::container_runtime::*;
use std::collections::HashMap;

fn script_set(name: &str) -> ContainerScriptSet {
    ContainerScriptSet {
        create: format!("{name}-create"),
        exec: format!("{name}-exec"),
        inspect: format!("{name}-inspect"),
        kill: format!("{name}-kill"),
    }
}

#[test]
fn parses_camel_case_container_script_alias_for_spawn_api() {
    let req = SpawnContainerRequest::parse(Some(&serde_json::json!({
        "mode": "new",
        "containerScript": "quecto-dev"
    })))
    .expect("camelCase spawn API should parse");

    assert_eq!(
        req,
        SpawnContainerRequest::New {
            repo: None,
            container_script: Some("quecto-dev".into())
        }
    );
}

#[test]
fn rejects_ambiguous_snake_and_camel_container_script_names() {
    let err = SpawnContainerRequest::parse(Some(&serde_json::json!({
        "mode": "new",
        "container_script": "quecto-dev",
        "containerScript": "api-dev"
    })))
    .unwrap_err();

    assert!(err.contains("container_script") && err.contains("containerScript"));
}

#[test]
fn requested_script_selection_returns_launch_commands() {
    let mut scripts = HashMap::new();
    scripts.insert("quecto-dev".into(), script_set("quecto-dev"));
    scripts.insert("api-dev".into(), script_set("api-dev"));
    let cfg = ContainerScriptsConfig {
        default: Some("quecto-dev".into()),
        scripts,
    };
    let req = SpawnContainerRequest::New {
        repo: None,
        container_script: Some("api-dev".into()),
    };

    let (name, set) = req.resolve_script(&cfg).unwrap().unwrap();

    assert_eq!(name, "api-dev");
    assert_eq!(set.create, "api-dev-create");
    assert_eq!(set.exec, "api-dev-exec");
}
