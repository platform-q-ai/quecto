//! Image attachments on `prompt` / `steer` / `follow_up` (#2422): the wire
//! field round-trips, every refusal refuses the whole command with its exact
//! message, and admitted images reach the provider on their own message,
//! whether the command runs at once or waits in the pending queue.

use super::dispatch_test_env::DispatchTestEnv as Env;
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role, UserImageBlock};
use crate::interface::cli::protocol::{AgentCommand, ImagePayload, StreamingBehavior};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A 2x2 PNG: signature, IHDR, IEND.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC";
/// A 2x2 JPEG: SOI, APP0 JFIF, SOF0, EOI.
const JPEG: &str = "/9j/4AAQSkZJRgABAQAAAQABAAD/wAALCAACAAIBAREA/9k=";

/// What one provider request carried as its latest user message.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    text: String,
    images: Vec<(String, String)>,
}

#[derive(Debug, Default)]
struct RecordingProvider {
    seen: Mutex<Vec<Seen>>,
}

impl LlmProvider for RecordingProvider {
    fn name(&self) -> &str {
        "recording"
    }

    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_owned()]
    }

    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let last_user = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .expect("a request carries a user message");
        self.seen.lock().unwrap().push(Seen {
            text: last_user.content.clone(),
            images: last_user
                .user_image_blocks
                .iter()
                .map(|b| (b.mime_type.clone(), b.data.clone()))
                .collect(),
        });
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("seen".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

/// A dispatch env whose model takes every image (#2421: a model with no
/// declared image input is sent markers instead).
fn recording_env() -> (Env, Arc<RecordingProvider>) {
    use crate::application::catalogue::dto::ModelLimits;
    use crate::application::catalogue::ports::ModelRuntime as _;
    use crate::domain::conversation::image_input::ImageInput;
    let provider = Arc::new(RecordingProvider::default());
    let mut env = Env::new(super::dispatch_test_env::make_workflow(), provider.clone());
    let limits = ModelLimits {
        image_input: ImageInput::AllImages,
        ..ModelLimits::default()
    };
    env.agent.apply_model("stub".to_owned(), limits);
    (env, provider)
}

fn image(mime: &str, data: &str) -> ImagePayload {
    ImagePayload::new(mime, data)
}

fn seen(text: &str, images: &[(&str, &str)]) -> Seen {
    Seen {
        text: text.into(),
        images: images
            .iter()
            .map(|(m, d)| (m.to_string(), d.to_string()))
            .collect(),
    }
}

fn command(kind: &str, message: &str, images: Vec<ImagePayload>) -> AgentCommand {
    let id = Some(format!("{kind}-1"));
    let message = message.to_string();
    match kind {
        "prompt" => AgentCommand::Prompt {
            id,
            message,
            images,
            streaming_behavior: None,
        },
        "steer" => AgentCommand::Steer {
            id,
            message,
            images,
        },
        "follow_up" => AgentCommand::FollowUp {
            id,
            message,
            images,
        },
        other => panic!("not a message command: {other}"),
    }
}

/// Dispatch `cmd` and return every event line it emitted.
async fn dispatch(env: &mut Env, cmd: AgentCommand) -> Vec<serde_json::Value> {
    let (tx, mut rx) = tokio::sync::broadcast::channel(256);
    {
        let mut ctx = env.ctx();
        ctx.broadcast_tx = Some(tx);
        super::uds_dispatch::dispatch_command(cmd, &mut ctx).await;
    }
    let mut events = Vec::new();
    while let Ok(line) = rx.try_recv() {
        events.push(serde_json::from_str(&line).unwrap());
    }
    events
}

fn response<'a>(events: &'a [serde_json::Value], id: &str) -> &'a serde_json::Value {
    events
        .iter()
        .find(|e| e["type"] == "response" && e["id"] == id)
        .unwrap_or_else(|| panic!("no response for {id}: {events:?}"))
}

#[test]
fn the_images_field_round_trips_and_stays_off_text_only_commands() {
    for kind in ["prompt", "steer", "follow_up"] {
        let wire = format!(
            r#"{{"type":"{kind}","message":"look","images":[{{"mimeType":"image/png","data":"{PNG}"}}]}}"#
        );
        let parsed: AgentCommand = serde_json::from_str(&wire).unwrap();
        let back = serde_json::to_value(&parsed).unwrap();
        assert_eq!(back["images"][0]["mimeType"], "image/png", "{kind}");
        assert_eq!(back["images"][0]["data"], PNG, "{kind}");

        let text_only: AgentCommand =
            serde_json::from_str(&format!(r#"{{"type":"{kind}","message":"look"}}"#)).unwrap();
        let back = serde_json::to_value(&text_only).unwrap();
        assert!(back.get("images").is_none(), "{kind}: {back}");
    }
}

#[tokio::test]
async fn an_idle_prompt_reaches_the_provider_with_its_images() {
    let (mut env, provider) = recording_env();
    let images = vec![image("image/png", PNG), image("image/jpeg", JPEG)];
    let events = dispatch(&mut env, command("prompt", "what is this?", images)).await;
    assert_eq!(response(&events, "prompt-1")["success"], true, "{events:?}");
    assert_eq!(
        provider.seen.lock().unwrap().as_slice(),
        [seen(
            "what is this?",
            &[("image/png", PNG), ("image/jpeg", JPEG)]
        )]
    );
}

#[tokio::test]
async fn an_images_only_prompt_runs() {
    let (mut env, provider) = recording_env();
    let events = dispatch(
        &mut env,
        command("prompt", "", vec![image("image/png", PNG)]),
    )
    .await;
    assert_eq!(response(&events, "prompt-1")["success"], true, "{events:?}");
    assert_eq!(
        provider.seen.lock().unwrap().as_slice(),
        [seen("", &[("image/png", PNG)])]
    );
}

#[tokio::test]
async fn an_idle_steer_and_follow_up_deliver_their_images() {
    for kind in ["steer", "follow_up"] {
        let (mut env, provider) = recording_env();
        dispatch(
            &mut env,
            command(kind, kind, vec![image("image/jpeg", JPEG)]),
        )
        .await;
        assert_eq!(
            provider.seen.lock().unwrap().as_slice(),
            [seen(kind, &[("image/jpeg", JPEG)])],
            "{kind}"
        );
    }
}

#[tokio::test]
async fn queued_steer_follow_up_and_prompt_deliver_their_images_with_their_message() {
    let (mut env, provider) = recording_env();
    env.session.set_streaming(true);
    let steer = command("steer", "steered", vec![image("image/png", PNG)]);
    let follow_up = command("follow_up", "followed", vec![image("image/jpeg", JPEG)]);
    let queued_prompt = AgentCommand::Prompt {
        id: None,
        message: "queued".into(),
        images: vec![image("image/png", PNG), image("image/png", PNG)],
        streaming_behavior: Some(StreamingBehavior::FollowUp),
    };
    for cmd in [follow_up, steer, queued_prompt] {
        dispatch(&mut env, cmd).await;
    }
    assert!(
        provider.seen.lock().unwrap().is_empty(),
        "nothing runs while busy"
    );

    env.session.set_streaming(false);
    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }
    assert_eq!(
        provider.seen.lock().unwrap().as_slice(),
        [
            seen("steered", &[("image/png", PNG)]),
            seen("followed", &[("image/jpeg", JPEG)]),
            seen("queued", &[("image/png", PNG), ("image/png", PNG)]),
        ]
    );
}

#[tokio::test]
async fn every_refusal_refuses_the_whole_command_with_its_exact_message() {
    let too_large = format!("{PNG}{}", "A".repeat(7 * 1024 * 1024));
    let cases: Vec<(Vec<ImagePayload>, &str)> = vec![
        (
            vec![image("image/png", PNG), image("image/svg+xml", PNG)],
            "images[1]: mimeType \"image/svg+xml\" is not allowed; use image/png, image/jpeg, image/gif or image/webp",
        ),
        (
            vec![image("image/png", "iVBORw0KGgo\nAAAANSUhEUg==")],
            "images[0]: data is not valid standard base64",
        ),
        (
            vec![image("image/png", JPEG)],
            "images[0]: data does not start with the image/png signature",
        ),
        (
            vec![image("image/png", &too_large)],
            "images[0]: image decodes to more than 3932160 bytes (3.75 MiB)",
        ),
        (
            vec![image("image/png", "iVBORw0KGgo=")],
            "images[0]: not a readable image/png image",
        ),
        (
            vec![image("image/png", PNG); 9],
            "too many images: 9; at most 8 per message",
        ),
    ];
    for kind in ["prompt", "steer", "follow_up"] {
        for (images, expected) in &cases {
            let (mut env, provider) = recording_env();
            let events = dispatch(&mut env, command(kind, "look", images.clone())).await;
            let refused = response(&events, &format!("{kind}-1"));
            assert_eq!(refused["success"], false, "{kind}: {events:?}");
            assert_eq!(refused["error"], *expected, "{kind}");
            assert_eq!(refused["command"], kind);
            assert_eq!(
                env.session.control_receipt_status(&format!("{kind}-1")),
                Some(crate::interface::cli::protocol::ControlStatus::Rejected),
                "{kind}: a refusal leaves its receipt"
            );
            assert!(provider.seen.lock().unwrap().is_empty(), "{kind} ran");
            assert!(env.messages.is_empty(), "{kind} appended a message");
            assert!(env.session.drain_pending().is_empty(), "{kind} queued");
            assert!(
                !serde_json::to_string(&events).unwrap().contains(PNG),
                "a refusal never echoes the image"
            );
        }
    }
}

#[tokio::test]
async fn a_busy_refusal_neither_queues_nor_runs() {
    let (mut env, provider) = recording_env();
    env.session.set_streaming(true);
    let events = dispatch(
        &mut env,
        command("follow_up", "later", vec![image("image/gif", PNG)]),
    )
    .await;
    assert_eq!(
        response(&events, "follow_up-1")["error"],
        "images[0]: data does not start with the image/gif signature"
    );
    assert!(env.session.drain_pending().is_empty());
    assert!(provider.seen.lock().unwrap().is_empty());
}

#[test]
fn only_a_steer_with_admissible_images_cancels_the_running_turn() {
    let steer = |images: &str| format!(r#"{{"type":"steer","message":"go","images":{images}}}"#);
    let good = format!(r#"[{{"mimeType":"image/png","data":"{PNG}"}}]"#);
    let bad = format!(r#"[{{"mimeType":"image/gif","data":"{PNG}"}}]"#);
    assert!(super::is_steer_command(&steer("[]")));
    assert!(super::is_steer_command(&steer(&good)));
    assert!(
        !super::is_steer_command(&steer(&bad)),
        "a steer dispatch will refuse must not cancel the turn first"
    );
    let prompt_steer =
        format!(r#"{{"type":"prompt","message":"go","streamingBehavior":"steer","images":{bad}}}"#);
    assert!(!super::is_steer_command(&prompt_steer));
}

fn with_images() -> Message {
    let mut message = Message::user("see attached");
    message.user_image_blocks = vec![
        UserImageBlock {
            mime_type: "image/png".into(),
            data: PNG.into(),
        },
        UserImageBlock {
            mime_type: "image/jpeg".into(),
            data: JPEG.into(),
        },
    ];
    message
}

#[test]
fn history_shows_how_many_images_a_message_carried_never_their_data() {
    let message = with_images();
    let json = crate::interface::cli::uds_session::message_to_json(&message);
    assert_eq!(json["imageCount"], 2);
    assert_eq!(
        json["imageMimeTypes"],
        serde_json::json!(["image/png", "image/jpeg"])
    );
    assert!(!json.to_string().contains(PNG));

    let plain = crate::interface::cli::uds_session::message_to_json(&Message::user("hi"));
    assert!(plain.get("imageCount").is_none(), "{plain}");
    assert!(plain.get("imageMimeTypes").is_none(), "{plain}");
}

#[test]
fn an_oversized_history_entry_keeps_the_image_summary() {
    let mut message = with_images();
    message.content = "x".repeat(crate::interface::cli::uds_session::HISTORY_PAGE_JSON_BUDGET + 1);
    let summary = crate::interface::cli::uds_session::message_to_json_for_history_page(&message);
    assert_eq!(summary["truncated"], true, "the fixture must be summarised");
    assert_eq!(summary["imageCount"], 2);
    assert_eq!(
        summary["imageMimeTypes"],
        serde_json::json!(["image/png", "image/jpeg"])
    );
    assert!(!summary.to_string().contains(PNG));
}

/// A control the reader refused (#2422) gets the same `Rejected` receipt
/// a dispatch refusal records.
#[tokio::test]
async fn a_control_the_reader_refused_records_its_rejected_receipt() {
    let (mut env, _provider) = recording_env();
    {
        let mut ctx = env.ctx();
        super::record_rejected(&mut ctx, "fwd-1", "steer").await;
    }
    assert_eq!(
        env.session.control_receipt_status("fwd-1"),
        Some(crate::interface::cli::protocol::ControlStatus::Rejected)
    );
}

/// Images a reader admitted run as admitted (#2422): dispatch does not
/// decode them again, and they reach the provider on their message.
#[tokio::test]
async fn a_steer_the_reader_admitted_runs_with_its_admitted_images() {
    let (mut env, provider) = recording_env();
    let line = format!(
        r#"{{"type":"steer","id":"s-9","message":"look","images":[{{"mimeType":"image/png","data":"{PNG}"}}]}}"#
    );
    let admitted = super::steer_images(&line).expect("an admissible steer");
    let cmd: AgentCommand = serde_json::from_str(&line).unwrap();
    {
        let mut ctx = env.ctx();
        super::dispatch_parsed(cmd, Some(admitted), &mut ctx).await;
    }
    assert_eq!(
        provider.seen.lock().unwrap().as_slice(),
        [seen("look", &[("image/png", PNG)])]
    );
}
