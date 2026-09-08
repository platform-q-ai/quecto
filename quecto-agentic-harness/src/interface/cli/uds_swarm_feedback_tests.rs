//! Real socket delivery into a workflow-free coordinator, followed by board action.
use super::dispatch_test_env::{DispatchTestEnv, make_workflow};
use crate::domain::message::{LlmResponse, ToolCall};
use crate::domain::tool::Tool;
use crate::infrastructure::tools::{
    swarm::{SwarmConfig, SwarmTool},
    swarm_bridge::SwarmContext,
};
use std::sync::Arc;

#[derive(Debug)]
struct ApprovalProvider {
    started: Arc<tokio::sync::Notify>,
}
impl crate::domain::provider::LlmProvider for ApprovalProvider {
    fn name(&self) -> &str {
        "approval-test"
    }
    fn chat(
        &self,
        request: crate::domain::provider::ChatRequest<'_>,
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
        let acted = request
            .messages
            .iter()
            .any(|m| m.role == crate::domain::message::Role::Tool);
        Box::pin(async move {
            Ok(LlmResponse {
                content: Some(
                    if acted {
                        "Acknowledged schema v2; resumed the blocked task."
                    } else {
                        ""
                    }
                    .into(),
                ),
                tool_calls: if acted {
                    vec![]
                } else {
                    vec![ToolCall {
                id:"apply-approval".into(), name:"swarm".into(),
                arguments: serde_json::json!({"op":"run","code":"from swarm import board; t=board.task(1); open('approved.txt','w').write('schema v2'); board.submit(1,t['token'],[{'artifact':'approved.txt','revision':'R2'}])"}).to_string(),
            }]
                },
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
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 120;
    board.create_run(&serde_json::json!({"goal":"wishlist","constraints":[],"criteria":[{"id":"tests","kind":"command","description":"pass"}],"member_limit":1,"deadline":deadline}), &crate::domain::swarm::ProcessIdentity { pid:std::process::id(), started:crate::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap() },None).unwrap();
    let tool = SwarmTool::new(
        workspace.clone(),
        Arc::new(crate::infrastructure::security::sandbox::Sandbox::new(
            Some(workspace.as_ref().clone()),
        )),
        SwarmConfig::default(),
    )
    .with_context(Some(board.clone()));
    let result = tool.execute(r#"{"op":"run","code":"from swarm import board; t=board.task_create('wishlist','wishlist',['approved schema']); c=board.claim(t['id']); board.block(t['id'],c['token'],'awaiting master approval')"}"#).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    env.agent.swap_registry(Box::new(registry));
    let socket = workspace.join("coordinator.sock");
    let mut ctx = env.ctx();
    ctx.workflow_state = None;
    ctx.workflow_config = None;
    let (commands, mut received) = tokio::sync::mpsc::channel(8);
    let (broadcast, _) = tokio::sync::broadcast::channel(16);
    let accept = crate::interface::cli::uds_multi::spawn_accept_loop(
        crate::interface::cli::uds_multi::AcceptLoopArgs {
            listener: tokio::net::UnixListener::bind(&socket).unwrap(),
            broadcast_tx: broadcast,
            cmd_tx: commands,
            cancel_handle: ctx.cancel_handle.clone(),
            turn_control: ctx.turn_control.clone(),
            live_clients: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            client_tool_registry: ctx.client_tool_registry.clone(),
            conversation_snapshot: ctx.conversation_snapshot.clone(),
            state_snapshot: ctx.state_snapshot.clone(),
            execution_state: ctx.execution_state.clone(),
            session_stats_snapshot: ctx.session_stats_snapshot.clone(),
            tool_catalogue_snapshot: ctx.tool_catalogue_snapshot.clone(),
            busy: ctx.busy.clone(),
            subagent_registry: None,
            workflow_state: None,
            workspace_path: workspace.as_ref().clone(),
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
    assert_eq!(board.summary().unwrap()["status"], "running");
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
            },
        ))
        .await
        .unwrap();
    let (broadcast, _) = tokio::sync::broadcast::channel(16);
    let (cancel, mut cancelled) = tokio::sync::oneshot::channel();
    *ctx.cancel_handle.lock().unwrap() = super::CancelSlot::Armed(cancel);
    let accept = crate::interface::cli::uds_multi::spawn_accept_loop(
        crate::interface::cli::uds_multi::AcceptLoopArgs {
            listener: tokio::net::UnixListener::bind(&socket).unwrap(),
            broadcast_tx: broadcast,
            cmd_tx: commands,
            cancel_handle: ctx.cancel_handle.clone(),
            turn_control: ctx.turn_control.clone(),
            live_clients: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            client_tool_registry: ctx.client_tool_registry.clone(),
            conversation_snapshot: ctx.conversation_snapshot.clone(),
            state_snapshot: ctx.state_snapshot.clone(),
            execution_state: ctx.execution_state.clone(),
            session_stats_snapshot: ctx.session_stats_snapshot.clone(),
            tool_catalogue_snapshot: ctx.tool_catalogue_snapshot.clone(),
            busy: ctx.busy.clone(),
            subagent_registry: None,
            workflow_state: None,
            workspace_path: workspace.as_ref().clone(),
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
