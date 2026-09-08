use super::*;

#[test]
fn swarm_cli_disables_default_workflow_and_rejects_explicit_activation() {
    for flag in [
        None,
        Some("--workflow"),
        Some("--workflow-guards"),
        Some("--workflow-spec"),
    ] {
        let mut args = vec!["--mode".to_string(), "uds".to_string()];
        if let Some(flag) = flag {
            args.push(flag.into());
            if flag == "--workflow-spec" {
                args.push("/unread/spec.json".into());
            }
        }
        let mut flags = super::super::parse_agent_flags(&args, &mut String::new()).unwrap();
        let result = disable_workflow(&mut flags);
        if flag.is_some() {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("workflow is unavailable for swarm agents")
            );
        } else {
            result.unwrap();
            assert!(flags.workflow_disabled);
            assert!(!flags.workflow_guards);
        }
    }
}
