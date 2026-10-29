//! Diagnostic orchestration for the deliberately bounded CI contract.
use super::{
    ADMISSION, COMPILED_COMMAND, COMPILED_NAME, SHAPE, WORKSPACE_CLIPPY,
    labeled_pull_request_trigger, mapping_keys_supported, required_commands, required_job,
    required_step, tolerance_is_required, unique_value, workflow_jobs,
};

pub(super) fn ci_contract_validation(workflow: &str) -> Result<(), String> {
    let lines: Vec<_> = workflow.lines().collect();
    let jobs = workflow_jobs(&lines)?;
    if labeled_pull_request_trigger(&lines) {
        // Admission is checked independently for both required jobs below.
    } else {
        return Err("unsupported pull_request trigger: require types: [labeled] and optional branches: [master]".to_owned());
    }
    for (id, display, required) in [
        (
            "static-quality",
            "Static Quality",
            vec![
                ("Strict workspace Clippy", vec![WORKSPACE_CLIPPY]),
                (
                    "Strict standalone quecto-tui Clippy",
                    vec!["cargo clippy -p quecto-tui --lib --bins -- -D warnings"],
                ),
                (
                    "Strict standalone quecto-agentic-harness Clippy",
                    vec!["cargo clippy -p quecto-agentic-harness --lib --bins -- -D warnings"],
                ),
            ],
        ),
        (
            "workspace-tests",
            "Workspace Tests",
            vec![(COMPILED_NAME, vec![SHAPE, COMPILED_COMMAND])],
        ),
    ] {
        let body = required_job(jobs, id)
            .ok_or_else(|| format!("missing or duplicate required job: {id}"))?;
        let supported = mapping_keys_supported(
            body,
            "    ",
            &[
                "name",
                "if",
                "runs-on",
                "timeout-minutes",
                "continue-on-error",
            ],
            &["steps", "needs", "permissions"],
        );
        if supported
            && unique_value(body, "    name: ") == Some(display)
            && unique_value(body, "    if: ") == Some(ADMISSION)
            && tolerance_is_required(body, "    continue-on-error: ")
            && body.iter().filter(|line| **line == "    steps:").count() == 1
        {
            let steps_start = body
                .iter()
                .position(|line| *line == "    steps:")
                .expect("unique required steps mapping");
            for (name, commands) in required {
                let step = required_step(&body[steps_start + 1..], name).ok_or_else(|| {
                    format!("job {id}: missing, malformed or duplicate step identity {name:?}")
                })?;
                if required_commands(&step, name, &commands) {
                    continue;
                }
                return Err(format!(
                    "job {id}, step {name:?}: unsupported mapping/env/tolerance/condition or missing, reordered or unsupported command"
                ));
            }
        } else {
            return Err(format!(
                "job {id}: unsupported mapping/env/defaults, display name, admission, tolerance or steps mapping"
            ));
        }
    }
    Ok(())
}
