use super::*;
use crate::application::agent_turn::declaration::TurnDeclarationService;
use crate::domain::turn_declaration::{TurnDeclaration, TurnIntent};
use serde_json::{Value, json};

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
    let tool = DeclareTurnOutcomeTool::new(service.clone(), role, id);
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
    ($($name:ident, $input:expr;)*) => {$(
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
    )*};
}
invalid! {
    malformed_json, "{".to_string();
    trailing_json, r#"{"outcome":"done","reason":"ready"}{}"#.to_string();
    null_document, "null".to_string();
    array_document, "[]".to_string();
    missing_outcome, r#"{"reason":"ready"}"#.to_string();
    missing_reason, r#"{"outcome":"done"}"#.to_string();
    null_outcome, r#"{"outcome":null,"reason":"ready"}"#.to_string();
    null_reason, r#"{"outcome":"done","reason":null}"#.to_string();
    unknown_enum, r#"{"outcome":"finished","reason":"ready"}"#.to_string();
    case_sensitive_enum, r#"{"outcome":"Done","reason":"ready"}"#.to_string();
    extra_field, r#"{"outcome":"done","reason":"ready","extra":true}"#.to_string();
    numeric_outcome, r#"{"outcome":1,"reason":"ready"}"#.to_string();
    numeric_reason, r#"{"outcome":"done","reason":1}"#.to_string();
    empty_reason, json!({"outcome":"done","reason":""}).to_string();
    whitespace_reason, json!({"outcome":"done","reason":" \t\u{2003} "}).to_string();
    cr_reason, json!({"outcome":"done","reason":"one\rtwo"}).to_string();
    lf_reason, json!({"outcome":"done","reason":"one\ntwo"}).to_string();
    crlf_reason, json!({"outcome":"done","reason":"one\r\ntwo"}).to_string();
    ascii_513, json!({"outcome":"done","reason":"a".repeat(513)}).to_string();
    multibyte_513, json!({"outcome":"done","reason":"界".repeat(513)}).to_string();
}

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
            let parent = DeclareTurnOutcomeTool::new(
                service.clone(),
                LaunchAuthority::TopLevelParent,
                identity(),
            );
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
    let parent =
        DeclareTurnOutcomeTool::new(service.clone(), LaunchAuthority::TopLevelParent, identity());
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
            let stale =
                DeclareTurnOutcomeTool::new(service.clone(), LaunchAuthority::TopLevelParent, old);
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
