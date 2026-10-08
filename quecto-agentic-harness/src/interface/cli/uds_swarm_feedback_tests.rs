//! Real socket delivery into a workflow-free coordinator, followed by board action.
use super::dispatch_test_env::{DispatchTestEnv, make_workflow};
use crate::application::tools::ports::Tool;
use crate::domain::conversation::value_objects::message::{LlmResponse, ToolCall};
use crate::infrastructure::tools::{swarm::SwarmTool, swarm_bridge::SwarmContext};
use std::sync::Arc;

#[derive(Debug)]
struct ApprovalProvider {
    started: Arc<tokio::sync::Notify>,
}
impl crate::application::providers::ports::LlmProvider for ApprovalProvider {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "approval-test"
    }
    fn chat(
        &self,
        request: crate::application::providers::ports::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<LlmResponse, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        let approved = request
            .messages
            .iter()
            .any(|m| m.content.contains("Approved: schema v2"));
        if !approved {
            self.started.notify_one();
            return Box::pin(std::future::pending());
        }
        // #2281: the approval applied one call at a time: read the task,
        // write the artifact, submit it under the task's token.
        let answers: Vec<&crate::domain::conversation::value_objects::message::Message> = request
            .messages
            .iter()
            .filter(|m| m.role == crate::domain::conversation::value_objects::message::Role::Tool)
            .collect();
        assert!(
            answers.iter().all(|m| !m.is_error),
            "an approval call was refused: {:?}",
            answers.iter().map(|m| &m.content).collect::<Vec<_>>()
        );
        let call = |id: &str, name: &str, arguments: serde_json::Value| ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.to_string(),
        };
        let tool_calls = match answers.as_slice() {
            [] => vec![call(
                "read-task",
                "swarm",
                serde_json::json!({"op":"task","task_id":1}),
            )],
            [_] => vec![call(
                "write-artifact",
                "write",
                serde_json::json!({"path":"approved.txt","content":"schema v2"}),
            )],
            [task, _] => {
                let task: serde_json::Value = serde_json::from_str(&task.content).unwrap();
                vec![call(
                    "apply-approval",
                    "swarm",
                    serde_json::json!({"op":"submit","task_id":1,"token":task["token"],
                        "evidence":[{"artifact":"approved.txt","revision":"R2"}]}),
                )]
            }
            _ => vec![],
        };
        let content = match tool_calls.is_empty() {
            true => "Acknowledged schema v2; resumed the blocked task.",
            false => "",
        };
        Box::pin(async move {
            Ok(LlmResponse {
                content: Some(content.into()),
                tool_calls,
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

#[tokio::test]
async fn master_steer_reaches_waiting_coordinator_and_resumes_blocked_task() {
    approval_exchange(false).await;
}

#[tokio::test]
async fn master_steer_interrupts_busy_coordinator_and_applies_approval() {
    approval_exchange(true).await;
}

async fn approval_exchange(busy: bool) {
    let started = Arc::new(tokio::sync::Notify::new());
    let mut env = DispatchTestEnv::new(
        make_workflow(),
        Arc::new(ApprovalProvider {
            started: started.clone(),
        }),
    );
    let workspace = Arc::new(env.tmp.path().to_path_buf());
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let board = SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 120;
    crate::infrastructure::tools::call_work::off_the_runtime(|| board.create_run(&serde_json::json!({"goal":"wishlist","constraints":[],"criteria":[{"id":"tests","kind":"command","description":"pass"}],"member_limit":1,"deadline":deadline}), &crate::domain::swarm::ProcessIdentity { pid:std::process::id(), started:crate::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap() },None)).unwrap();
    let tool = SwarmTool::new().with_context(Some(board.clone()));
    let op = |request: serde_json::Value| {
        let tool = &tool;
        async move {
            let result = tool.execute(&request.to_string()).await.unwrap();
            assert!(!result.is_error, "{request}: {}", result.content);
            serde_json::from_str::<serde_json::Value>(&result.content).unwrap()
        }
    };
    let task = op(
        serde_json::json!({"op":"task_create","request":"wishlist","title":"wishlist",
        "acceptance":["approved schema"]}),
    )
    .await;
    let claim = op(serde_json::json!({"op":"claim","task_id":task["id"]})).await;
    op(
        serde_json::json!({"op":"block","task_id":task["id"],"token":claim["token"],
        "reason":"awaiting master approval"}),
    )
    .await;
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    registry.register(Arc::new(
        crate::infrastructure::tools::filesystem::WriteTool::new(
            workspace.clone(),
            Arc::new(crate::infrastructure::security::sandbox::Sandbox::new(
                Some(workspace.as_ref().clone()),
            )),
        ),
    ));
    env.agent.swap_registry(Box::new(registry));
    let socket = workspace.join("coordinator.sock");
    let mut ctx = env.ctx();
    ctx.workflow_state = None;
    ctx.workflow_config = None;
    let (commands, mut received) = tokio::sync::mpsc::channel(8);
    let (broadcast, _) = tokio::sync::broadcast::channel(16);
    let accept = crate::interface::cli::uds_multi::spawn_accept_loop(
        crate::interface::cli::uds_multi::AcceptLoopArgs {
            extensions: Default::default(),
            listener: tokio::net::UnixListener::bind(&socket).unwrap(),
            broadcast_tx: broadcast,
            cmd_tx: commands,
            disconnect_tx: tokio::sync::mpsc::unbounded_channel().0,
            cancel_handle: ctx.cancel_handle.clone(),
            turn_control: ctx.turn_control.clone(),
            live_clients: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            client_tool_registry: ctx.client_tool_registry.clone(),
            session: ctx.sessions.clone(),
            runtime_store: Default::default(),
            state_snapshot: ctx.state_snapshot.clone(),
            execution_state: ctx.execution_state.clone(),
            session_stats_snapshot: ctx.session_stats_snapshot.clone(),
            tool_catalogue_snapshot: ctx.tool_catalogue_snapshot.clone(),
            busy: ctx.busy.clone(),
            subagent_registry: None,
            workflow_state: None,
            workspace_path: workspace.as_ref().clone(),
            teardown: None,
        },
    );
    let delivery = tokio::spawn(async move {
        if busy {
            started.notified().await;
        }
        crate::infrastructure::tools::subagent_registry::send_subagent_uds_command_with_timeout(
            &socket, r#"{"type":"prompt","streamingBehavior":"steer","message":"Approved: schema v2","ack":"accept","id":"approval-1"}"#,
            std::time::Duration::from_secs(2)).await.unwrap()
    });
    if busy {
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            super::handle_prompt(
                &mut ctx,
                super::PromptCommand {
                    id: None,
                    type_name: "prompt".into(),
                    message: "Waiting for master feedback".into(),
                    streaming_behavior: None,
                },
            ),
        )
        .await
        .expect("steer must cancel the in-flight provider call");
    }
    let reply = delivery.await.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&reply).unwrap()["success"],
        true
    );
    let command = tokio::time::timeout(std::time::Duration::from_secs(2), received.recv())
        .await
        .unwrap()
        .unwrap();
    let crate::interface::cli::uds_multi::ClientMessage::Command(command) = command else {
        panic!("expected approval command")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&command.line).unwrap()["id"],
        serde_json::from_str::<serde_json::Value>(&reply).unwrap()["id"]
    );
    let super::LineResult::Command(command) = super::parse_line(&command.line) else {
        panic!("expected parsed command")
    };
    super::dispatch_command(command, &mut ctx).await;
    assert!(
        ctx.messages
            .iter()
            .any(|m| m.content.contains("Acknowledged schema v2"))
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("approved.txt")).unwrap(),
        "schema v2"
    );
    let summary =
        crate::infrastructure::tools::call_work::off_the_runtime(|| board.summary()).unwrap();
    assert_eq!(summary["status"], "running", "{summary}");
    // The approval was applied on the board: the blocked task is submitted.
    assert_eq!(summary["tasks"][0]["status"], "submitted", "{summary}");
    accept.abort();
}

#[tokio::test]
async fn rejected_socket_steer_does_not_cancel_but_explicit_abort_does() {
    let mut env = DispatchTestEnv::new(
        make_workflow(),
        Arc::new(ApprovalProvider {
            started: Arc::new(tokio::sync::Notify::new()),
        }),
    );
    let workspace = Arc::new(env.tmp.path().to_path_buf());
    let socket = workspace.join("full.sock");
    let ctx = env.ctx();
    let (commands, _received) = tokio::sync::mpsc::channel(1);
    commands
        .send(crate::interface::cli::uds_multi::ClientMessage::Command(
            crate::interface::cli::uds_multi::ClientCommand {
                line: "occupied".into(),
                client_id: 0,
                admitted: None,
            },
        ))
        .await
        .unwrap();
    let (broadcast, _) = tokio::sync::broadcast::channel(16);
    let (cancel, mut cancelled) = tokio::sync::oneshot::channel();
    *ctx.cancel_handle.lock().unwrap() = super::CancelSlot::Armed(cancel);
    let accept = crate::interface::cli::uds_multi::spawn_accept_loop(
        crate::interface::cli::uds_multi::AcceptLoopArgs {
            extensions: Default::default(),
            listener: tokio::net::UnixListener::bind(&socket).unwrap(),
            broadcast_tx: broadcast,
            cmd_tx: commands,
            disconnect_tx: tokio::sync::mpsc::unbounded_channel().0,
            cancel_handle: ctx.cancel_handle.clone(),
            turn_control: ctx.turn_control.clone(),
            live_clients: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            client_tool_registry: ctx.client_tool_registry.clone(),
            session: ctx.sessions.clone(),
            runtime_store: Default::default(),
            state_snapshot: ctx.state_snapshot.clone(),
            execution_state: ctx.execution_state.clone(),
            session_stats_snapshot: ctx.session_stats_snapshot.clone(),
            tool_catalogue_snapshot: ctx.tool_catalogue_snapshot.clone(),
            busy: ctx.busy.clone(),
            subagent_registry: None,
            workflow_state: None,
            workspace_path: workspace.as_ref().clone(),
            teardown: None,
        },
    );

    let rejected = crate::infrastructure::tools::subagent_registry::send_subagent_uds_command_with_timeout(
        &socket, r#"{"type":"prompt","streamingBehavior":"steer","message":"unretained approval","ack":"accept","id":"full"}"#, std::time::Duration::from_secs(2)).await.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rejected).unwrap()["success"],
        false
    );
    assert!(
        matches!(
            cancelled.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "rejected steer cancelled active work"
    );
    assert!(!ctx.turn_control.is_steer_pending());
    let aborted =
        crate::infrastructure::tools::subagent_registry::send_subagent_uds_command_with_timeout(
            &socket,
            r#"{"type":"abort","ack":"accept","id":"stop"}"#,
            std::time::Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&aborted).unwrap()["success"],
        true
    );
    assert!(ctx.turn_control.is_abort_pending());
    assert!(!matches!(
        cancelled.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    accept.abort();
}

/// #2403 review H1/M5: a prompt or a steer that reaches an idle agent is
/// marked a prompt; a swarm wake that reaches an idle member is the
/// harness's, unmarked, so the member's real task keeps its pin.
#[tokio::test]
async fn an_idle_wake_is_unmarked_and_a_prompt_or_steer_is_a_prompt() {
    use crate::domain::conversation::value_objects::user_kind::UserKind;
    use crate::interface::cli::uds_swarm_control::SWARM_WAKE;
    for (type_name, id, expected) in [
        ("prompt", Some("p"), UserKind::Prompt),
        ("steer", Some("s"), UserKind::Prompt),
        (SWARM_WAKE, None, UserKind::Unmarked),
    ] {
        let mut env = DispatchTestEnv::with_unselected_workflow();
        let mut ctx = env.ctx();
        let text = format!("sent as {type_name}");
        super::handle_prompt(
            &mut ctx,
            super::PromptCommand {
                id: id.map(str::to_string),
                type_name: type_name.into(),
                message: text.clone().into(),
                streaming_behavior: None,
            },
        )
        .await;
        let sent = ctx
            .messages
            .iter()
            .find(|message| message.content == text)
            .unwrap_or_else(|| panic!("{type_name}: the message ran"));
        assert_eq!(sent.user_kind, expected, "{type_name}");
    }
}
