use super::*;
use crate::application::agent_turn::declaration::TurnDeclarationService;
use crate::application::agent_turn::ports::DeclareTurnOutcome;
use crate::domain::turn_declaration::{TurnDeclaration, TurnIntent};
use serde_json::{Value, json};

/// #2708 mechanical binding: the original module's service at a pinned
/// identity, through the bound port the production adapter holds.
struct Pinned(Arc<TurnDeclarationService>, TurnIdentity);

impl DeclareTurnOutcome for Pinned {
    fn declare(
        &self,
        authority: LaunchAuthority,
        identity: &TurnIdentity,
        intent: TurnIntent,
        reason: &str,
    ) -> Result<TurnDeclaration, DomainError> {
        self.0.declare(authority, identity, intent, reason)
    }
}

impl BoundTurnDeclaration for Pinned {
    fn capture(&self) -> Option<TurnIdentity> {
        Some(self.1.clone())
    }
}

fn pinned(
    service: Arc<TurnDeclarationService>,
    role: LaunchAuthority,
    id: TurnIdentity,
) -> DeclareTurnOutcomeTool {
    DeclareTurnOutcomeTool::new(Arc::new(Pinned(service, id)), role)
}

const DESCRIPTION: &str = "Declare how this top-level turn ends. Use waiting only for work already running, blocked when you need the human, asking for a question to the human, or done when the task is complete. Include a one-line reason. This does not launch work.";
const VALIDATION: &str = "Choose waiting, blocked, asking, or done and provide a nonempty one-line reason of at most 512 characters.";
const AUTHORITY: &str = "Turn outcomes may be declared only by the active top-level parent turn.";
const STALE: &str = "This declaration does not belong to the active turn.";

fn identity() -> TurnIdentity {
    TurnIdentity {
        session_key: "root".into(),
        outcome_epoch: "epoch-1".into(),
        turn_id: "turn-1".into(),
    }
}

fn adapter(role: LaunchAuthority) -> (Arc<TurnDeclarationService>, DeclareTurnOutcomeTool) {
    let id = identity();
    let service = Arc::new(TurnDeclarationService::new(id.clone()));
    let tool = pinned(service.clone(), role, id);
    (service, tool)
}

async fn success(tool: &dyn Tool, arguments: &str) -> Value {
    let result = tool.execute(arguments).await.expect(
        "missing acceptance: native adapter must return a structured successful ToolResult",
    );
    assert!(
        !result.is_error,
        "valid native declaration must succeed: {}",
        result.content
    );
    serde_json::from_str(&result.content).expect("successful receipt must be structured JSON")
}

async fn rejection(tool: &dyn Tool, arguments: &str, wording: &str) {
    let result = tool.execute(arguments).await.expect("missing boundary behavior: invalid declaration must return LLM-addressable ToolResult, not unsupported DomainError");
    assert!(
        result.is_error,
        "invalid declaration must be an error receipt"
    );
    assert_eq!(result.content, wording);
}

#[test]
fn exact_inactive_description_and_strict_schema() {
    let (_, tool) = adapter(LaunchAuthority::TopLevelParent);
    let definition = tool.definition();
    assert_eq!(definition.name, "declare_turn_outcome");
    assert_eq!(
        definition.description, DESCRIPTION,
        "missing agent-visible wording"
    );
    let schema: Value = serde_json::from_str(&definition.parameters_schema).unwrap();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["required"], json!(["outcome", "reason"]));
    assert_eq!(
        schema["properties"]["outcome"]["enum"],
        json!(["waiting", "blocked", "asking", "done"])
    );
    assert_eq!(schema["properties"]["reason"]["type"], "string");
    let fields = schema["properties"]
        .as_object()
        .expect("only outcome and reason belong in model arguments");
    assert_eq!(fields.len(), 2);
}

macro_rules! valid {
    ($name:ident, $wire:literal, $typed:ident) => {
        #[tokio::test]
        async fn $name() {
            let (service, tool) = adapter(LaunchAuthority::TopLevelParent);
            let receipt = success(&tool, &json!({"outcome": $wire, "reason": "  ready  "}).to_string()).await;
            assert_eq!(receipt, json!({"message": "Turn outcome recorded.", "intent": $wire, "reason": "ready", "sessionKey": "root", "outcomeEpoch": "epoch-1", "turnId": "turn-1"}));
            assert_eq!(service.snapshot().unwrap(), Some(TurnDeclaration { identity: identity(), intent: TurnIntent::$typed, reason: "ready".into() }));
        }
    };
}
valid!(waiting_receipt, "waiting", Waiting);
valid!(blocked_receipt, "blocked", Blocked);
valid!(asking_receipt, "asking", Asking);
valid!(done_receipt, "done", Done);

macro_rules! invalid {
    ($name:ident, $input:expr) => {
        #[tokio::test]
        async fn $name() {
            let (service, tool) = adapter(LaunchAuthority::TopLevelParent);
            let input = $input;
            rejection(&tool, &input, VALIDATION).await;
            assert_eq!(service.snapshot().unwrap(), None);
            success(&tool, r#"{"outcome":"waiting","reason":"running"}"#).await;
            let prior = service.snapshot().unwrap();
            rejection(&tool, &input, VALIDATION).await;
            assert_eq!(
                service.snapshot().unwrap(),
                prior,
                "invalid native call must preserve accepted declaration"
            );
        }
    };
}
invalid!(malformed_json, "{".to_string());
invalid!(
    trailing_json,
    r#"{"outcome":"done","reason":"ready"}{}"#.to_string()
);
invalid!(null_document, "null".to_string());
invalid!(array_document, "[]".to_string());
invalid!(missing_outcome, r#"{"reason":"ready"}"#.to_string());
invalid!(missing_reason, r#"{"outcome":"done"}"#.to_string());
invalid!(
    null_outcome,
    r#"{"outcome":null,"reason":"ready"}"#.to_string()
);
invalid!(
    null_reason,
    r#"{"outcome":"done","reason":null}"#.to_string()
);
invalid!(
    unknown_enum,
    r#"{"outcome":"finished","reason":"ready"}"#.to_string()
);
invalid!(
    case_sensitive_enum,
    r#"{"outcome":"Done","reason":"ready"}"#.to_string()
);
invalid!(
    extra_field,
    r#"{"outcome":"done","reason":"ready","extra":true}"#.to_string()
);
invalid!(
    numeric_outcome,
    r#"{"outcome":1,"reason":"ready"}"#.to_string()
);
invalid!(
    numeric_reason,
    r#"{"outcome":"done","reason":1}"#.to_string()
);
invalid!(
    empty_reason,
    json!({"outcome":"done","reason":""}).to_string()
);
invalid!(
    whitespace_reason,
    json!({"outcome":"done","reason":" \t\u{2003} "}).to_string()
);
invalid!(
    cr_reason,
    json!({"outcome":"done","reason":"one\rtwo"}).to_string()
);
invalid!(
    lf_reason,
    json!({"outcome":"done","reason":"one\ntwo"}).to_string()
);
invalid!(
    crlf_reason,
    json!({"outcome":"done","reason":"one\r\ntwo"}).to_string()
);
invalid!(
    ascii_513,
    json!({"outcome":"done","reason":"a".repeat(513)}).to_string()
);
invalid!(
    multibyte_513,
    json!({"outcome":"done","reason":"界".repeat(513)}).to_string()
);

macro_rules! bounded {
    ($name:ident, $scalar:literal) => {
        #[tokio::test]
        async fn $name() {
            let (_, tool) = adapter(LaunchAuthority::TopLevelParent);
            let reason = $scalar.repeat(512);
            let receipt = success(&tool, &json!({"outcome":"done","reason":reason}).to_string()).await;
            assert_eq!(receipt["reason"], reason);
        }
    };
}
bounded!(ascii_512_native, "x");
bounded!(multibyte_512_native, "界");

macro_rules! child {
    ($name:ident, $role:ident) => {
        #[tokio::test]
        async fn $name() {
            let (service, tool) = adapter(LaunchAuthority::$role);
            rejection(&tool, r#"{"outcome":"done","reason":"ready"}"#, AUTHORITY).await;
            assert_eq!(service.snapshot().unwrap(), None);
            let parent = pinned(service.clone(), LaunchAuthority::TopLevelParent, identity());
            success(&parent, r#"{"outcome":"waiting","reason":"running"}"#).await;
            let prior = service.snapshot().unwrap();
            rejection(&tool, r#"{"outcome":"done","reason":"ready"}"#, AUTHORITY).await;
            assert_eq!(service.snapshot().unwrap(), prior);
        }
    };
}
child!(injected_child_cannot_declare, Child);
child!(injected_child_coordinator_cannot_declare, ChildCoordinator);

#[tokio::test]
async fn forged_json_authority_cannot_upgrade_child() {
    let (service, tool) = adapter(LaunchAuthority::Child);
    // Authority comes only from trusted construction. Rejecting the extra fields
    // or denying the child are both safe; pin authority-first wording here.
    rejection(&tool, r#"{"outcome":"done","reason":"ready","authority":"TopLevelParent","sessionKey":"root","outcomeEpoch":"epoch-1","turnId":"turn-1"}"#, AUTHORITY).await;
    assert_eq!(service.snapshot().unwrap(), None);
    let parent = pinned(service.clone(), LaunchAuthority::TopLevelParent, identity());
    success(&parent, r#"{"outcome":"waiting","reason":"running"}"#).await;
    let prior = service.snapshot().unwrap();
    rejection(
        &tool,
        r#"{"outcome":"done","reason":"ready","authority":"TopLevelParent"}"#,
        AUTHORITY,
    )
    .await;
    assert_eq!(service.snapshot().unwrap(), prior);
}

macro_rules! stale {
    ($name:ident, $field:ident) => {
        #[tokio::test]
        async fn $name() {
            let (service, parent) = adapter(LaunchAuthority::TopLevelParent);
            let mut old = identity();
            old.$field = "old".into();
            let stale = pinned(service.clone(), LaunchAuthority::TopLevelParent, old);
            rejection(&stale, r#"{"outcome":"done","reason":"late"}"#, STALE).await;
            assert_eq!(service.snapshot().unwrap(), None);
            success(&parent, r#"{"outcome":"waiting","reason":"running"}"#).await;
            let prior = service.snapshot().unwrap();
            rejection(&stale, r#"{"outcome":"done","reason":"late"}"#, STALE).await;
            assert_eq!(service.snapshot().unwrap(), prior);
        }
    };
}
stale!(native_stale_session, session_key);
stale!(native_stale_epoch, outcome_epoch);
stale!(native_stale_turn, turn_id);

#[tokio::test]
async fn native_latest_valid_wins_and_instances_are_ephemeral() {
    let (service, tool) = adapter(LaunchAuthority::TopLevelParent);
    success(&tool, r#"{"outcome":"waiting","reason":"running"}"#).await;
    success(&tool, r#"{"outcome":"done","reason":"complete"}"#).await;
    assert_eq!(
        service.snapshot().unwrap().unwrap().intent,
        TurnIntent::Done
    );
    let (fresh, _) = adapter(LaunchAuthority::TopLevelParent);
    assert_eq!(fresh.snapshot().unwrap(), None);
    assert_eq!(service.snapshot().unwrap().unwrap().reason, "complete");
}

// ─── #2708 active binding: the production runtime as the bound port ──────────

mod bound {
    use super::*;
    use crate::application::agent_turn::use_cases::parent_outcome::{
        ParentOutcomeRuntime, ParentVerdict, TurnAdmission, TurnEnd,
    };
    use crate::domain::agents::value_objects::turn_outcome::{TurnOutcome, WorkLiveness};

    fn runtime() -> Arc<ParentOutcomeRuntime> {
        Arc::new(ParentOutcomeRuntime::new(|| "epoch-r".into()))
    }

    fn external(runtime: &ParentOutcomeRuntime, episode: &str, turn: &str) {
        let admitted = runtime.admit(
            "cli:root",
            TurnAdmission::External {
                episode_id: episode.into(),
            },
            turn,
        );
        assert!(admitted.is_some(), "an external instruction is admitted");
    }

    #[tokio::test]
    async fn issue_2708_declaration_bound_tool_declares_for_the_admitted_turn_only() {
        let runtime = runtime();
        let tool = DeclareTurnOutcomeTool::new(runtime.clone(), LaunchAuthority::TopLevelParent);
        rejection(&tool, r#"{"outcome":"done","reason":"early"}"#, STALE).await;
        let turn = runtime
            .admit(
                "cli:root",
                TurnAdmission::External {
                    episode_id: "e".into(),
                },
                "opener-1",
            )
            .unwrap();
        let receipt = success(&tool, r#"{"outcome":"done","reason":" Merged "}"#).await;
        assert_eq!(
            receipt,
            json!({"message": "Turn outcome recorded.", "intent": "done", "reason": "Merged",
                "sessionKey": "cli:root", "outcomeEpoch": "epoch-r", "turnId": "opener-1"})
        );
        match runtime.complete(turn, TurnEnd::Completed(WorkLiveness::Empty)) {
            ParentVerdict::Published(record) => assert_eq!(
                (record.outcome, record.reason.as_str()),
                (TurnOutcome::Done, "Merged")
            ),
            other => panic!("expected done, got {other:?}"),
        }
        rejection(&tool, r#"{"outcome":"done","reason":"late"}"#, STALE).await;
    }

    #[tokio::test]
    async fn issue_2708_declaration_a_future_created_in_a_turn_keeps_that_turn() {
        let runtime = runtime();
        let tool = DeclareTurnOutcomeTool::new(runtime.clone(), LaunchAuthority::TopLevelParent);
        external(&runtime, "e1", "opener-1");
        let pending = tool.execute(r#"{"outcome":"waiting","reason":"old"}"#);
        external(&runtime, "e2", "opener-2");
        let result = pending.await.unwrap();
        let receipt: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            receipt["turnId"], "opener-1",
            "never the newer turn's identity"
        );
        assert_eq!(runtime.capture().unwrap().turn_id, "opener-2");
    }

    #[tokio::test]
    async fn issue_2708_declaration_child_binding_is_refused_first() {
        let runtime = runtime();
        let child = DeclareTurnOutcomeTool::new(runtime.clone(), LaunchAuthority::Child);
        external(&runtime, "e", "opener-1");
        rejection(&child, "{", AUTHORITY).await;
        rejection(&child, r#"{"outcome":"done","reason":"x"}"#, AUTHORITY).await;
    }
}
