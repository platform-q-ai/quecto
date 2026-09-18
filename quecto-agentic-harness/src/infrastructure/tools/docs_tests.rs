use super::*;

#[test]
fn lookup_doc_resolves_plain_md_and_prefixed_names() {
    assert!(lookup_doc("quick-start").is_none());
    assert!(lookup_doc("subagents").is_some());
    assert!(lookup_doc("config").is_some());
    assert!(lookup_doc("subagents.md").is_some());
    assert!(lookup_doc("docs/subagents.md").is_some());
    assert!(lookup_doc("docs/docs-tool-embeds/workflow.md").is_some());
    assert!(lookup_doc("  MODELS  ").is_some());
    assert!(lookup_doc("quecto").is_none());
    assert!(lookup_doc("readme").is_none());
    assert!(lookup_doc("uds-protocol").is_none());
    assert!(lookup_doc("sessions").is_none());
    assert!(lookup_doc("contributor-cookbooks").is_none());
    assert!(lookup_doc("nope").is_none());
}

#[test]
fn doc_title_reads_first_h1() {
    assert_eq!(doc_title("# Hello world\n\nbody"), Some("Hello world"));
    assert_eq!(doc_title("no title\n## Section"), None);
}

#[test]
fn default_constructs_the_parent_docs_tool() {
    let tool: DocsTool = Default::default();
    assert_eq!(tool.definition().name.as_ref(), "docs");
    assert!(!tool.definition().description.is_empty());
}

#[tokio::test]
async fn execute_without_name_lists_toc_with_titles() {
    let tool = DocsTool::new();
    let result = tool.execute("{}").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("operating manual"));
    assert!(result.content.contains("Table of contents:"));
    assert!(!result.content.contains("quick-start — "));
    assert!(result.content.contains("Workflow"));
    assert!(result.content.contains("subagents — "));
    assert!(result.content.contains("workflow — "));
    assert!(result.content.contains("extensions — "));
    assert!(result.content.contains("models — "));
    assert!(!result.content.contains("contributor-cookbooks"));
    assert!(!result.content.contains("uds-protocol"));
}

#[tokio::test]
async fn execute_with_name_returns_doc_body() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"workflow"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Workflow"));
    assert!(result.content.contains("workflow"));
}

#[tokio::test]
async fn execute_returns_concise_subagents_deep_dive() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"subagents"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("get_messages"));
    assert!(result.content.contains("read_only"));
    // Must stay a deep-dive, not the old full manual.
    assert!(result.content.len() < 8_000);
}

#[tokio::test]
async fn execute_accepts_md_suffix_and_docs_prefix() {
    let tool = DocsTool::new();
    let result = tool
        .execute(r#"{"name":"docs/workflow.md"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Workflow"));
}

#[tokio::test]
async fn execute_unknown_doc_is_error_and_lists_toc() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"nonexistent"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("workflow"));
    assert!(result.content.contains("Table of contents:"));
}

#[tokio::test]
async fn execute_with_invalid_json_lists_docs() {
    let tool = DocsTool::new();
    let result = tool.execute("not json").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("operating manual"));
}

/// Removed quick-start is neither advertised nor retrievable by either role.
#[tokio::test]
async fn quick_start_is_removed_for_all_agents() {
    for tool in [DocsTool::new(), DocsTool::for_child_content()] {
        assert!(!tool.definition().description.contains("quick-start"));
        assert!(!tool.definition().parameters_schema.contains("quick-start"));
        let toc = tool.execute("{}").await.unwrap();
        assert!(!toc.content.contains("quick-start"));
        for name in [
            "quick-start",
            "quick-start.md",
            "docs/quick-start.md",
            "docs/docs-tool-embeds/quick-start.md",
            "QUICK-START",
        ] {
            assert!(lookup_doc(name).is_none());
            let result = tool
                .execute(&format!(r#"{{"name":"{name}"}}"#))
                .await
                .unwrap();
            assert!(result.is_error);
            assert!(result.content.contains("No embedded doc named"));
        }
    }
}

/// #1319: non-parent pages remain readable for spawned children.
#[tokio::test]
async fn spawned_can_read_other_manual_pages() {
    let tool = DocsTool::for_child_content();
    let result = tool.execute(r#"{"name":"workflow"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("# Workflow"));
}

#[test]
fn subagents_embed_teaches_container_environments() {
    let doc = lookup_doc("subagents").expect("subagents embed");
    for needle in [
        "Container spawning",
        "container: true",
        "container_config",
        // #2024 S4c: the agent-facing menu is the roster line and the
        // get_container_configs command; the operator command stays in
        // the container-runtime embed.
        "Available container configs:",
        "get_container_configs",
        "fresh clone of the config's `--repo`",
        "quecto container doctor",
        "config set --local container_configs",
        "sandbox",
        "\"mode\":\"existing\"",
        "environment_ref=C1",
        "get_containers",
        "kill_container",
        "absolute path",
        "effective configuration",
        "container_config=<name>",
        "without `--config`",
        "never inside a container child",
        "`quecto config trust`",
    ] {
        assert!(doc.contains(needle), "subagents embed misses {needle}");
    }
    let runtime = lookup_doc("container-runtime").expect("container-runtime embed");
    for needle in [
        "## How to find configs and refs",
        "get_container_configs",
        "config get --effective container_configs",
        "\"source\":\"overlay\"|\"global\"",
        "get_containers",
        "fresh clone",
    ] {
        assert!(
            runtime.contains(needle),
            "container-runtime embed misses {needle}"
        );
    }
    let swarm = lookup_doc("swarm").expect("swarm embed");
    for needle in [
        "## Which container",
        "the container the coordinator was spawned into",
        "get_container_configs",
        "Workers are spawned with `container` omitted",
        "cannot host a swarm",
    ] {
        assert!(swarm.contains(needle), "swarm embed misses {needle}");
    }
}

#[tokio::test]
async fn embedded_swarm_manual_teaches_types_limits_and_terminal_reporting() {
    let result = DocsTool::for_child_content()
        .execute(r#"{"name":"swarm"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    for required in [
        "list[str]",
        "RLIMIT_NPROC",
        "bash",
        "accepted",
        "op=summary",
        "workspace-relative",
    ] {
        assert!(result.content.contains(required), "manual lacks {required}");
    }
}

/// #1707: admission help is discoverable and embedded for every agent role.
#[tokio::test]
async fn admission_broker_manual_is_discoverable_and_actionable() {
    for tool in [DocsTool::new(), DocsTool::for_child_content()] {
        let toc = tool.execute("{}").await.unwrap();
        assert!(toc.content.contains("admission-broker — Admission broker"));
        for name in ["admission-broker", "docs/admission-broker.md"] {
            let result = tool
                .execute(&format!(r#"{{"name":"{name}"}}"#))
                .await
                .unwrap();
            assert!(!result.is_error, "{}", result.content);
            assert_eq!(Some(result.content.as_str()), lookup_doc(name));
            for required in [
                "disabled by default",
                "max_scopes",
                "1024",
                "terminal_capacity",
                "4096",
                "openai-api",
                "openai-oauth",
                "queue_timeout_ms",
                "attempt_timeout_ms",
                "quecto admission-broker run",
                "quecto admission-broker status",
                "quecto admission-broker reset",
                "journal_healthy",
                "uncertain",
                "get_state",
                "timer",
                "parallel",
                "get_messages",
                "min_interval_ms",
                "\"tool_uses\"",
                "\"recipient_name\": \"functions.spawn\"",
                "left-panel",
                "monotonic",
                "rebases",
                "#1708",
                // #2024 S3 recovery story: roots re-register, children are
                // respawned, and the parent is told how it sees that.
                "Recovery: reset and broker restart",
                "its parent must respawn it",
                "authorityStatus: \"unavailable\"",
                "restart required",
                "RestartPreventExitStatus=3",
            ] {
                assert!(result.content.contains(required), "manual lacks {required}");
            }
        }
    }
}

/// #2024 S5: the `setup` index is the first page of the manual and routes
/// every situation to one area page, one goal command, one verification
/// and one rollback — all of them concrete commands.
#[tokio::test]
async fn setup_index_is_first_and_routes_every_area() {
    let tool = DocsTool::new();
    let toc = tool.execute("{}").await.unwrap();
    let first = toc
        .content
        .lines()
        .find(|line| line.starts_with("- "))
        .expect("table of contents lists pages");
    assert!(
        first.starts_with("- setup — Setting up quecto"),
        "setup must be listed first: {first}"
    );
    assert!(tool.definition().description.contains("setup"));
    let doc = lookup_doc("setup").expect("setup embed");
    assert!(doc.len() < 10_000, "setup index is {} B", doc.len());
    for needle in [
        "quecto auth login --provider",
        "quecto auth status",
        "quecto config set agents.defaults.model",
        "quecto config get --effective agents.defaults.model",
        "quecto config unset agents.defaults.model",
        "quecto config set --global admission",
        "quecto admission-broker install-service",
        "quecto admission-broker status",
        "quecto admission-broker uninstall-service",
        "quecto container init",
        "quecto container doctor",
        "quecto container status",
        "quecto config unset --local container_configs.standard",
        "docs {\"name\": \"config\"}",
        "docs {\"name\": \"models\"}",
        "docs {\"name\": \"admission-broker\"}",
        "docs {\"name\": \"container-runtime\"}",
        "docs {\"name\": \"swarm\"}",
        "--show-secrets",
        "QUECTO_BASE_DIR",
    ] {
        assert!(doc.contains(needle), "setup index misses {needle}");
    }
}

/// #2024 S5: the four area pages are runbooks of one fixed shape, so an
/// agent that follows one literally always knows what to run, what to
/// expect, and how to undo it.
#[test]
fn area_pages_share_the_runbook_shape() {
    for name in ["config", "models", "admission-broker", "container-runtime"] {
        let doc = lookup_doc(name).expect("area embed");
        for heading in [
            "## Preconditions",
            "## Do",
            "## Verify",
            "## Rollback",
            "## If it fails",
        ] {
            assert!(doc.contains(heading), "{name} embed lacks {heading}");
        }
        assert!(
            !doc.contains("container-config-trust"),
            "{name} names the retired container trust record"
        );
    }
    let models = lookup_doc("models").expect("models embed");
    for needle in [
        "quecto auth login --provider openai --token",
        "quecto auth login --provider anthropic --token",
        "--oauth",
        "--device-code",
        "quecto auth status",
        "quecto auth logout --provider",
        "quecto models discover",
        "<base_dir>/models.json",
        "quecto config set agents.defaults.model",
        "quecto config set --global agents.defaults.model",
        "\"persist\":\"local\"",
    ] {
        assert!(models.contains(needle), "models embed misses {needle}");
    }
    let config = lookup_doc("config").expect("config embed");
    for needle in [
        "quecto config trust",
        "quecto config get --local",
        "quecto config get --global",
        "Overlay: ",
        "(trusted)",
        "(untrusted)",
        "config-overlay-trust.json",
        "global-only",
    ] {
        assert!(config.contains(needle), "config embed misses {needle}");
    }
    let container = lookup_doc("container-runtime").expect("container-runtime embed");
    for needle in [
        "quecto container init",
        "podman build -t quecto-box:local",
        "quecto container status",
        "quecto container doctor",
        "\"container\":true",
        "get_containers",
        "kill_container",
        "init --refresh",
        "## Trust boundary",
        "## Upgrades",
    ] {
        assert!(
            container.contains(needle),
            "container-runtime embed misses {needle}"
        );
    }
    let admission = lookup_doc("admission-broker").expect("admission-broker embed");
    for needle in [
        "quecto config set --global admission",
        "quecto admission-broker install-service --dry-run",
        "quecto admission-broker uninstall-service",
        "quecto config unset --global admission",
        "\"bindings\":{\"*\":\"account\"}",
        "systemctl --user status quecto-admission-broker.service",
        "not running for directory",
    ] {
        assert!(
            admission.contains(needle),
            "admission-broker embed misses {needle}"
        );
    }
}
