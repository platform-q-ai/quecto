//! #2216: a spawn that asks for workflow mode the child could never use is
//! refused up front, before anything launches.
use std::collections::BTreeMap;

use crate::application::tools::ports::Tool;
use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::tools::inherited_tool_policy::workflow_tool_identity;
use crate::infrastructure::tools::spawn::SpawnTool;

const SPEC: &str = r#"{"template":{"id":"rev","label":"Rev","description":"d","steps":[{"key":"a","label":"A","phase":"review"}]}}"#;

fn spawn_with(workflow: Option<ProfileAvailabilityScope>, by_name: bool) -> SpawnTool {
    let tool = SpawnTool::new(vec![]);
    let mut tools = BTreeMap::from([("read".to_string(), ProfileAvailabilityScope::Both)]);
    if let Some(scope) = workflow {
        let key = if by_name {
            "workflow".to_string()
        } else {
            workflow_tool_identity().stable_id.into_owned()
        };
        tools.insert(key, scope);
    }
    tool.set_inherited_child_policy_snapshot_for_spawn(tools);
    tool
}

fn requests() -> [String; 3] {
    [
        r#"{"agent_id":"a","workflow":true}"#.to_string(),
        r#"{"agent_id":"a","workflow":true,"workflow_guards":true}"#.to_string(),
        format!(r#"{{"agent_id":"a","workflow_spec":{SPEC}}}"#),
    ]
}

#[test]
fn workflow_request_is_admitted_when_the_parent_never_built_the_tool() {
    let tool = spawn_with(None, false);
    for request in requests() {
        tool.parse_args_for_test(&request)
            .unwrap_or_else(|e| panic!("{request}: {e}"));
    }
}

#[test]
fn workflow_request_is_admitted_when_the_parent_allows_it_to_children() {
    for scope in [
        ProfileAvailabilityScope::Both,
        ProfileAvailabilityScope::Child,
    ] {
        let tool = spawn_with(Some(scope), false);
        for request in requests() {
            assert!(tool.parse_args_for_test(&request).is_ok(), "{scope:?}");
        }
    }
}

#[test]
fn workflow_request_is_refused_when_the_parent_denies_it_to_children() {
    for scope in [
        ProfileAvailabilityScope::None,
        ProfileAvailabilityScope::Parent,
    ] {
        for by_name in [false, true] {
            let tool = spawn_with(Some(scope), by_name);
            for request in requests() {
                let error = tool.parse_args_for_test(&request).unwrap_err();
                assert!(
                    error.contains("workflow is not available to children of this agent"),
                    "{scope:?} {request}: {error}"
                );
            }
            assert!(
                tool.parse_args_for_test(r#"{"agent_id":"a"}"#).is_ok(),
                "a plain spawn is unaffected"
            );
        }
    }
}

#[test]
fn workflow_request_is_refused_when_disable_tools_names_the_workflow_tool() {
    let tool = spawn_with(None, false);
    let identity = workflow_tool_identity();
    for denial in [
        "workflow".to_string(),
        identity.stable_id.to_string(),
        identity.legacy_name_id.to_string(),
    ] {
        let request = format!(r#"{{"agent_id":"a","workflow":true,"disable_tools":["{denial}"]}}"#);
        let error = tool.parse_args_for_test(&request).unwrap_err();
        assert!(
            error.contains("disable_tools denies the workflow tool"),
            "{denial}: {error}"
        );
    }
    assert!(
        tool.parse_args_for_test(r#"{"agent_id":"a","disable_tools":["workflow"]}"#)
            .is_ok(),
        "disabling workflow without asking for it is fine"
    );
}
