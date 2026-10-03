//! Images survive a session reload (#2424), through the real file store as
//! composition builds it: a tool result's and a user message's images come
//! back verbatim from their sidecars (the provider request is byte for byte
//! the one before), an image is stored once however many messages carry it,
//! a missing, corrupt or unreadable sidecar keeps its reference (never a load
//! failure, never a lost image), and deleting the session removes its
//! sidecars.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::application::sessions::ports::SessionStore;
use crate::domain::conversation::image_input::{GifVerdicts, ImageInput, SentConversation};
use crate::domain::conversation::stored_images::{ImageKind, unavailable_marker};
use crate::domain::message::{Message, ToolCall, UserImageBlock};
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;
use crate::domain::tool::ImageBlock;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::providers::anthropic::AnthropicProvider;

use quecto_image::{ImageMime, samples};

/// Real images, whose base64 admission takes.
fn png_bytes() -> Vec<u8> {
    samples::png(2, 3)
}

fn jpeg_bytes() -> Vec<u8> {
    samples::jpeg(2, 2)
}

fn id() -> SessionIdentity {
    SessionIdentity::from_persisted_key("cli:images")
}

fn base64(bytes: &[u8]) -> String {
    samples::encode(bytes)
}

/// The sidecar name of an image whose text is `text`.
fn sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn store(dir: &Path) -> FileSessionStore {
    crate::composition::sessions::build_file_session_store(dir)
}

fn images_dir(dir: &Path) -> PathBuf {
    FlatSessionLayout::new(dir).image_dir(&id())
}

fn transcript(dir: &Path) -> String {
    std::fs::read_to_string(FlatSessionLayout::new(dir).session_file(&id())).unwrap()
}

/// `read` of an image whose text is `text`, as the agent loop records it.
fn read_image_exchange(text: &str) -> Vec<Message> {
    let call = ToolCall {
        id: "call-1".into(),
        name: "read".into(),
        arguments: r#"{"path":"shot.png"}"#.into(),
    };
    let mut result = Message::tool("call-1", "Read image file [image/png] (30 B)");
    result.tool_name = Some("read".into());
    result.image_blocks = vec![ImageBlock::new("image/png", text)];
    vec![
        Message::user("look at the screenshot"),
        Message::assistant("", vec![call]),
        result,
        Message::assistant("a red square", vec![]),
    ]
}

/// A user message with an admitted image.
fn user_with_image(text: &str, mime: &str, image: &str) -> Message {
    let mime = ImageMime::parse_exact(mime).expect("an image type");
    let image = UserImageBlock::restore(mime, image.to_string()).expect("admitted");
    Message::user(text).with_user_images(vec![image])
}

fn session(messages: Vec<Message>) -> Session {
    Session {
        key: id(),
        messages,
        workflow_run: None,
        subagent_roster: Vec::new(),
    }
}

async fn reload(dir: &Path) -> Session {
    store(dir).load(&id()).await.unwrap().expect("saved")
}

/// The messages as a model that takes every image is sent them.
fn sent_text(messages: &[Message]) -> Vec<String> {
    let mut messages = messages.to_vec();
    let verdicts = GifVerdicts::default();
    let sent = SentConversation::new(&mut messages, "m", ImageInput::AllImages, &verdicts);
    sent.messages().iter().map(|m| m.content.clone()).collect()
}

/// The messages as the Anthropic adapter serializes them for a request.
fn request(messages: &[Message]) -> String {
    let mut messages = messages.to_vec();
    let verdicts = GifVerdicts::default();
    let sent = SentConversation::new(&mut messages, "m", ImageInput::AllImages, &verdicts);
    let (_, body) = AnthropicProvider::build_messages_public(sent.messages());
    serde_json::to_string(&body).unwrap()
}

#[tokio::test]
async fn a_tool_result_image_survives_a_reload_and_the_record_holds_only_its_reference() {
    let tmp = TempDir::new().unwrap();
    let png = base64(&png_bytes());
    store(tmp.path())
        .save(&session(read_image_exchange(&png)))
        .await
        .unwrap();

    let loaded = reload(tmp.path()).await;
    let images = &loaded.messages[2].image_blocks;
    assert_eq!(images.len(), 1, "the tool result's image is back");
    assert_eq!(images[0].mime_type, "image/png");
    assert_eq!(images[0].data(), png);
    assert_eq!(
        loaded.messages[2].content,
        "Read image file [image/png] (30 B)"
    );

    let record = transcript(tmp.path());
    assert!(record.contains(&sha256(&png)), "the record names the image");
    assert!(
        !record.contains(&png),
        "a reference, not the image: {record}"
    );
    assert_eq!(
        std::fs::read_to_string(images_dir(tmp.path()).join(sha256(&png))).unwrap(),
        png,
        "the sidecar holds the image's text under its SHA-256"
    );
}

#[tokio::test]
async fn a_user_image_survives_a_reload() {
    let tmp = TempDir::new().unwrap();
    store(tmp.path())
        .save(&session(vec![
            user_with_image("what is this?", "image/jpeg", &base64(&jpeg_bytes())),
            Message::assistant("a photo", vec![]),
        ]))
        .await
        .unwrap();

    let loaded = reload(tmp.path()).await;
    let images = &loaded.messages[0].user_image_blocks;
    assert_eq!(images.len(), 1, "the user's image is back");
    assert_eq!(images[0].mime_type(), "image/jpeg");
    assert_eq!(images[0].data(), base64(&jpeg_bytes()));
    assert_eq!(loaded.messages[0].content, "what is this?");
}

#[tokio::test]
async fn a_reloaded_request_is_byte_identical_for_padded_unpadded_and_wrapped_images() {
    let padded = base64(&png_bytes());
    let unpadded = padded.trim_end_matches('=').to_string();
    let wrapped = format!("{}\r\n{}", &padded[..20], &padded[20..]);
    assert_ne!(padded, unpadded, "the image's base64 is padded");
    let strict = base64(&jpeg_bytes());
    for text in [padded, unpadded, wrapped] {
        let tmp = TempDir::new().unwrap();
        // A tool result keeps any text; a user's image is strict base64.
        let mut messages = read_image_exchange(&text);
        messages.push(user_with_image("and this?", "image/jpeg", &strict));
        let before = request(&messages);
        store(tmp.path()).save(&session(messages)).await.unwrap();
        let after = request(&reload(tmp.path()).await.messages);
        assert_eq!(after, before, "{text:?}");
    }
}

/// #2422 saved an images-only prompt's text as `[image]`; with the images
/// themselves saved, the text is kept exactly as sent (empty, or blank), so
/// the request after a reload is the one before, byte for byte.
#[tokio::test]
async fn an_images_only_prompt_reloads_byte_identical_with_its_text_as_sent() {
    for text in ["", "  \n\t "] {
        let tmp = TempDir::new().unwrap();
        let messages = vec![
            user_with_image(text, "image/png", &base64(&png_bytes())),
            Message::assistant("a small image", vec![]),
        ];
        let before = request(&messages);
        store(tmp.path()).save(&session(messages)).await.unwrap();
        let loaded = reload(tmp.path()).await;
        assert_eq!(loaded.messages[0].content, text, "saved as sent");
        assert_eq!(request(&loaded.messages), before, "{text:?}");
    }
}

#[tokio::test]
async fn a_user_image_strict_admission_refuses_is_kept_unloaded_not_dropped() {
    let tmp = TempDir::new().unwrap();
    // Stored as it was held; strict re-admission refuses it on restore.
    let unadmitted = UserImageBlock::unchecked_for_tests(ImageMime::Png, "iVBORw0K");
    let held = Message::user("look").with_user_images(vec![unadmitted]);
    store(tmp.path())
        .save(&session(vec![held, Message::assistant("ok", vec![])]))
        .await
        .unwrap();
    let loaded = reload(tmp.path()).await;
    assert!(loaded.messages[0].user_image_blocks.is_empty());
    assert_eq!(
        loaded.messages[0].unloaded_images.len(),
        1,
        "never silently gone"
    );
    assert_eq!(
        sent_text(&loaded.messages)[0],
        format!("look\n{}", marker("iVBORw0K"))
    );
    assert!(transcript(tmp.path()).contains(&sha256("iVBORw0K")));
}

#[tokio::test]
async fn an_image_appended_by_a_delta_save_survives_a_reload() {
    let tmp = TempDir::new().unwrap();
    let files = store(tmp.path());
    let mut messages = vec![Message::user("hi"), Message::assistant("hello", vec![])];
    files.save(&session(messages.clone())).await.unwrap();
    messages.extend(read_image_exchange(&base64(&png_bytes())));
    files
        .save_clean_delta(&id(), &messages, 2, None)
        .await
        .unwrap();

    let loaded = reload(tmp.path()).await;
    assert_eq!(
        loaded.messages[4].image_blocks[0].data(),
        base64(&png_bytes())
    );
}

#[tokio::test]
async fn the_same_image_in_two_messages_is_stored_once() {
    let tmp = TempDir::new().unwrap();
    let png = base64(&png_bytes());
    let mut messages = read_image_exchange(&png);
    messages.push(user_with_image("and again", "image/png", &png));
    store(tmp.path()).save(&session(messages)).await.unwrap();

    let names: Vec<String> = std::fs::read_dir(images_dir(tmp.path()))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec![sha256(&png)], "one sidecar for one image");
    let loaded = reload(tmp.path()).await;
    assert_eq!(loaded.messages[2].image_blocks[0].data(), png);
    assert_eq!(loaded.messages[4].user_image_blocks[0].data(), png);
}

#[tokio::test]
async fn a_second_save_hashes_no_image_again() {
    let tmp = TempDir::new().unwrap();
    let files = store(tmp.path());
    let mut messages = read_image_exchange(&base64(&png_bytes()));
    messages.push(user_with_image(
        "again",
        "image/jpeg",
        &base64(&jpeg_bytes()),
    ));
    let mut saved = session(messages);
    files.save(&saved).await.unwrap();
    saved.messages.push(Message::user("more"));
    files.save(&saved).await.unwrap();
    files
        .save_clean_delta(&id(), &saved.messages, saved.messages.len() - 1, None)
        .await
        .unwrap();
    assert_eq!(
        saved.messages[2].image_blocks[0].digest_builds_for_tests(),
        1
    );
}

/// The marker a request shows for the image whose text is `text`.
fn marker(text: &str) -> String {
    unavailable_marker(&sha256(text))
}

#[tokio::test]
async fn a_missing_sidecar_keeps_its_reference_and_a_request_shows_a_marker() {
    let tmp = TempDir::new().unwrap();
    let png = base64(&png_bytes());
    store(tmp.path())
        .save(&session(read_image_exchange(&png)))
        .await
        .unwrap();
    std::fs::remove_file(images_dir(tmp.path()).join(sha256(&png))).unwrap();

    let loaded = reload(tmp.path()).await;
    let message = &loaded.messages[2];
    assert!(message.image_blocks.is_empty());
    assert_eq!(message.content, "Read image file [image/png] (30 B)");
    assert_eq!(message.unloaded_images.len(), 1);
    assert_eq!(message.unloaded_images[0].kind, ImageKind::Tool);
    let sent = sent_text(&loaded.messages);
    assert_eq!(
        sent[2],
        format!("Read image file [image/png] (30 B)\n{}", marker(&png))
    );
    // Saved again, the reference stays named and the marker is never written.
    store(tmp.path()).save(&loaded).await.unwrap();
    let record = transcript(tmp.path());
    assert!(record.contains(&sha256(&png)), "{record}");
    assert!(!record.contains("image unavailable"), "{record}");
}

#[tokio::test]
async fn an_image_unreadable_for_a_while_is_back_on_the_next_load() {
    let tmp = TempDir::new().unwrap();
    let png = base64(&png_bytes());
    store(tmp.path())
        .save(&session(read_image_exchange(&png)))
        .await
        .unwrap();
    let sidecar = images_dir(tmp.path()).join(sha256(&png));
    std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&sidecar).is_ok() {
        println!("skipped: mode 000 does not deny this user (root?), so there is no fault");
        return;
    }

    let loaded = reload(tmp.path()).await;
    assert_eq!(
        loaded.messages[2].unloaded_images.len(),
        1,
        "kept, not lost"
    );
    // A new store's first save rewrites the whole transcript: a compaction.
    store(tmp.path()).save(&loaded).await.unwrap();
    std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o600)).unwrap();

    let back = reload(tmp.path()).await;
    assert!(sidecar.is_file(), "a reference not read is still live");
    assert_eq!(back.messages[2].image_blocks[0].data(), png);
    assert!(back.messages[2].unloaded_images.is_empty());
}

#[tokio::test]
async fn a_corrupt_sidecar_keeps_its_reference_and_a_save_of_the_image_repairs_it() {
    let tmp = TempDir::new().unwrap();
    let png = base64(&png_bytes());
    let original = session(vec![
        user_with_image("", "image/png", &png),
        Message::assistant("seen", vec![]),
    ]);
    store(tmp.path()).save(&original).await.unwrap();
    // The same length, other bytes.
    let sidecar = images_dir(tmp.path()).join(sha256(&png));
    std::fs::write(&sidecar, "A".repeat(png.len())).unwrap();

    let loaded = reload(tmp.path()).await;
    let message = &loaded.messages[0];
    assert!(
        message.user_image_blocks.is_empty(),
        "a hash mismatch is never sent"
    );
    assert_eq!(message.content, "", "nothing is written into the text");
    assert_eq!(sent_text(&loaded.messages)[0], marker(&png));

    // A store that still holds the image (another process's view) saves it.
    store(tmp.path()).save(&original).await.unwrap();
    assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), png);
    assert_eq!(
        reload(tmp.path()).await.messages[0].user_image_blocks[0].data(),
        png
    );
}

#[tokio::test]
async fn an_image_the_store_cannot_keep_is_named_and_sent_as_a_marker() {
    let tmp = TempDir::new().unwrap();
    // A tool result's image of a type quecto does not admit (an extension's).
    let svg = base64(b"<svg/>");
    let mut drawn = Message::tool("call-1", "draw");
    drawn.image_blocks = vec![ImageBlock::new("image/svg+xml", svg.clone())];
    store(tmp.path())
        .save(&session(vec![drawn, Message::assistant("ok", vec![])]))
        .await
        .unwrap();
    assert!(!images_dir(tmp.path()).exists(), "not stored");
    let loaded = reload(tmp.path()).await;
    assert_eq!(
        loaded.messages[0].unloaded_images.len(),
        1,
        "never silently gone"
    );
    assert_eq!(
        sent_text(&loaded.messages)[0],
        format!("draw\n{}", marker(&svg))
    );
}

#[tokio::test]
async fn a_transcript_only_shown_reads_no_sidecar() {
    let tmp = TempDir::new().unwrap();
    let png = base64(&png_bytes());
    let files = store(tmp.path());
    files
        .save(&session(read_image_exchange(&png)))
        .await
        .unwrap();
    let shown = files.load_transcript(&id()).await.unwrap().unwrap();
    assert!(shown.messages[2].image_blocks.is_empty(), "no sidecar read");
    assert_eq!(shown.messages[2].unloaded_images.len(), 1, "but named");
    assert_eq!(
        shown.messages[2].unloaded_images[0].reference.sha256,
        sha256(&png)
    );
}

#[tokio::test]
async fn deleting_the_session_removes_its_sidecars() {
    let tmp = TempDir::new().unwrap();
    let files = store(tmp.path());
    let png = base64(&png_bytes());
    files
        .save(&session(read_image_exchange(&png)))
        .await
        .unwrap();
    assert!(images_dir(tmp.path()).join(sha256(&png)).is_file());

    files.save(&Session::new(id())).await.unwrap();

    assert!(!files.exists(&id()).await.unwrap(), "the session is gone");
    assert!(!images_dir(tmp.path()).exists(), "and so are its sidecars");
}

#[tokio::test]
async fn a_store_composed_without_sidecars_names_images_and_stores_none() {
    let tmp = TempDir::new().unwrap();
    let bare = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    bare.save(&session(read_image_exchange(&base64(&png_bytes()))))
        .await
        .unwrap();
    assert!(!images_dir(tmp.path()).exists());
    let loaded = bare.load(&id()).await.unwrap().unwrap();
    assert_eq!(loaded.messages[2].unloaded_images.len(), 1);
}

#[tokio::test]
async fn a_record_written_before_images_were_stored_loads_as_before() {
    let tmp = TempDir::new().unwrap();
    let path = FlatSessionLayout::new(tmp.path()).session_file(&id());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "{\"type\":\"snapshot\",\"key\":\"cli:images\",\"messages\":[\
         {\"role\":\"user\",\"content\":\"old\"},\
         {\"role\":\"tool\",\"content\":\"Read image file [image/png] (30 B)\",\"tool_call_id\":\"c\"}]}\n",
    )
    .unwrap();

    let loaded = reload(tmp.path()).await;
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(
        loaded.messages[1].content,
        "Read image file [image/png] (30 B)"
    );
    assert!(loaded.messages[1].image_blocks.is_empty());
    assert!(loaded.messages[1].unloaded_images.is_empty());
}

#[tokio::test]
async fn a_text_only_session_writes_no_image_field_and_no_sidecar_directory() {
    let tmp = TempDir::new().unwrap();
    store(tmp.path())
        .save(&session(vec![
            Message::user("hi"),
            Message::assistant("hello", vec![]),
        ]))
        .await
        .unwrap();

    let record = transcript(tmp.path());
    assert!(!record.contains("\"images\""), "{record}");
    assert!(!record.contains("\"user_images\""), "{record}");
    assert!(!images_dir(tmp.path()).exists());
}
