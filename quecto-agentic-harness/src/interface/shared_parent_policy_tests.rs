use super::*;

#[test]
fn parent_prompt_contains_only_role_and_routing_guidance() {
    let result = build_system_prompt(&None, false);
    assert!(!result.contains("Current date and time:"));
    assert!(result.contains(agent_role_preamble()));
    assert!(result.contains("Parent Agent"));
    for excluded in [
        "`docs` tool",
        "operating manual",
        "quick-start",
        "definitive source",
        "name `quecto`",
        "quecto-tui",
        "quecto-api",
        "quecto-mcp",
    ] {
        assert!(!result.contains(excluded));
    }
    for expected in [
        "## Parent Software Development Orchestration Playbook:",
        "Always configure `member_limit` to 25",
        "task-appropriate fixed pool up front",
        "rather than filling capacity",
        "### Swarm completion and cleanup",
    ] {
        assert!(result.contains(expected));
    }
}

/// Given either prompt builder, the approved playbook belongs only to the parent.
#[test]
fn parent_playbook_contract_is_excluded_from_children() {
    let requirements = [
        "communication, scope, orchestration, approvals, and synthesis",
        "ALL",
        "small, localized, low-risk",
        "clear requirements and files",
        "no substantial investigation, planning, or independent review",
        "brief edit and focused verification",
        "ordinary child for convenience",
        "Planning must not roll into delivery",
        "epics, issues, acceptance criteria, and dependencies",
        "features, refactors, bugfixes, and chores",
        "independent investigation",
        "independent verification",
        "PR review",
        "fresh boards and separate containers/checkouts",
        "collaboration, not independent review",
        "Swarms cannot use workflow",
        "issue/PR links",
        "exact branch and SHA",
        "before consulting delivery reasoning",
        "revision-bound",
        "recheck affected changes",
        "platform-q-ai/quecto only",
        "merge-requested",
        "resets on failure",
    ];
    for parent in [
        build_system_prompt(&None, false),
        build_agent_system_prompt(None, None, false, ""),
    ] {
        for requirement in requirements {
            assert!(
                parent.contains(requirement),
                "missing approved parent policy: {requirement}"
            );
        }
        assert!(!parent.contains("prefer a child with `workflow: true`"));
        assert!(!parent.contains("Handle directly in the parent when the task is focused"));
    }
    for child in [
        build_system_prompt(&None, true),
        build_agent_system_prompt(None, None, true, ""),
    ] {
        assert!(child.starts_with(child_role_preamble()));
        assert!(!child.contains(agent_role_preamble()));
        assert!(!child.contains(parent_coordination_policy()));
        for requirement in requirements {
            assert!(
                !child.contains(requirement),
                "parent policy leaked to child: {requirement}"
            );
        }
    }
}

/// User review amendments replace earlier policy, without leaking into children.
#[test]
fn parent_playbook_applies_user_review_amendments() {
    let additions = [
        "while you, the parent, remain available to the user",
        "Prefer swarms for nearly all software development and team based work.",
        "Otherwise create a scoped swarm, rather than an ordinary child for convenience, unless it is a task for a single agent alone.",
        "Unless an assigned Epic or Issue contains full instructions, handoffs must include requirements",
        "Ensure swarms understand that the `merge-requested` label is required to start/restart relevant CI and resets on failure.",
    ];
    let removals = [
        "Read delegated reports and critical evidence",
        "Before replacing work",
        "When blocked",
        "Missing evidence is a blocker",
        "### Completion and learning",
        "Completion requires all applicable gates",
        "From corrections",
        "Persist approved lessons",
        "Inspect labels, PR head, and workflows",
        "Apply the label when ready",
        "Confirm the actual run",
        "After failure, diagnose",
        "push alone",
        "If the label is present",
        "Avoid blind label",
        "Treat this as a repository-scoped rule",
    ];
    for parent in [
        build_system_prompt(&None, false),
        build_agent_system_prompt(None, None, false, ""),
    ] {
        for text in additions {
            assert!(parent.contains(text), "missing user amendment: {text}");
        }
        for text in removals {
            assert!(!parent.contains(text), "removed policy remains: {text}");
        }
        assert!(parent.contains("### Repository CI: platform-q-ai/quecto only"));
    }
    for child in [
        build_system_prompt(&None, true),
        build_agent_system_prompt(None, None, true, ""),
    ] {
        for text in additions {
            assert!(!child.contains(text), "parent amendment leaked: {text}");
        }
    }
}

/// A successor must receive durable evidence, not merely an in-conversation summary.
#[test]
fn parent_handoff_requires_persisted_accessible_artifacts_and_successor_references() {
    let artifact_policy = "Persist handoffs and evidence as durable, accessible artifacts; pass their references to successor swarms and confirm those swarms can access them.";
    for parent in [
        build_system_prompt(&None, false),
        build_agent_system_prompt(None, None, false, ""),
    ] {
        let handoff = parent
            .split_once("### Handoff and evidence\n")
            .expect("parent has a handoff policy")
            .1
            .split_once("\n### Repository CI")
            .expect("handoff policy has a section boundary")
            .0;
        assert!(
            handoff.contains(artifact_policy),
            "handoffs must persist evidence and transfer accessible artifact references"
        );
    }
    for child in [
        build_system_prompt(&None, true),
        build_agent_system_prompt(None, None, true, ""),
    ] {
        assert!(!child.contains(artifact_policy));
        assert!(!child.contains("### Handoff and evidence"));
    }
}
