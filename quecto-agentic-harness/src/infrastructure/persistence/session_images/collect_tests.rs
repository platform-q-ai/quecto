//! Garbage collection of image sidecars (#2424), through the real file
//! store: a compaction removes the sidecars its transcript no longer names
//! (session memory's references are information only, so an image a cut
//! archived goes too), and an append removes nothing.
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::application::sessions::ports::{ContextSpillStore, SessionStore};
use crate::domain::conversation::stored_images::{ImageRef, sha256_hex};
use crate::domain::message::{Message, UserImageBlock};
use crate::domain::session::{Session, SpillEntry};
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;

const PNG: &str = "iVBORw0KGgofirst";
const GIF: &str = "R0lGODlhsecond";

fn id() -> SessionIdentity {
    SessionIdentity::from_persisted_key("cli:collect")
}

fn with_image(text: &str, mime: &str, image: &str) -> Message {
    let mut message = Message::user(text);
    let mime = quecto_image::ImageMime::parse_exact(mime).expect("an image type");
    // Collection needs no admitted image: any text names a sidecar.
    message.user_image_blocks = vec![UserImageBlock::unchecked_for_tests(mime, image)];
    message
}

fn session(messages: Vec<Message>) -> Session {
    Session {
        key: id(),
        messages,
        workflow_run: None,
        subagent_roster: Vec::new(),
    }
}

fn sidecar(dir: &Path, image: &str) -> PathBuf {
    FlatSessionLayout::new(dir)
        .image_dir(&id())
        .join(sha256_hex(image.as_bytes()))
}

fn two_images() -> Vec<Message> {
    vec![
        with_image("first", "image/png", PNG),
        Message::assistant("a png", vec![]),
        with_image("second", "image/gif", GIF),
        Message::assistant("a gif", vec![]),
    ]
}

#[tokio::test]
async fn a_compaction_removes_the_sidecars_its_transcript_no_longer_names() {
    let tmp = TempDir::new().unwrap();
    let store = crate::composition::sessions::build_file_session_store(tmp.path());
    store.save(&session(two_images())).await.unwrap();
    assert!(sidecar(tmp.path(), PNG).is_file() && sidecar(tmp.path(), GIF).is_file());

    // A rewind to the second prompt: the prefix changed, so the file compacts.
    store
        .save(&session(two_images().split_off(2)))
        .await
        .unwrap();

    assert!(!sidecar(tmp.path(), PNG).exists(), "nothing names the png");
    assert!(sidecar(tmp.path(), GIF).is_file(), "the gif is still named");
}

#[tokio::test]
async fn an_image_only_session_memory_names_is_collected() {
    let tmp = TempDir::new().unwrap();
    let layout = FlatSessionLayout::new(tmp.path());
    let store = crate::composition::sessions::build_file_session_store(tmp.path());
    store.save(&session(two_images())).await.unwrap();
    // The first prompt was archived to session memory with its image.
    FileContextSpillStore::new(layout)
        .append(
            &id(),
            &SpillEntry {
                id: "turn0:msg:user".into(),
                tool: "user".into(),
                input_preview: "first".into(),
                tokens: 1,
                content: "first".into(),
                images: vec![ImageRef {
                    sha256: sha256_hex(PNG.as_bytes()),
                    mime_type: "image/png".into(),
                }],
            },
        )
        .await
        .unwrap();

    store
        .save(&session(two_images().split_off(2)))
        .await
        .unwrap();

    assert!(
        !sidecar(tmp.path(), PNG).exists(),
        "session memory's reference is information only: recall never restores it"
    );
}

#[tokio::test]
async fn only_a_compaction_collects() {
    let tmp = TempDir::new().unwrap();
    let store = crate::composition::sessions::build_file_session_store(tmp.path());
    let mut messages = two_images();
    store.save(&session(messages.clone())).await.unwrap();
    // A sidecar no record names, as a crash between its write and its
    // record's leaves one.
    let stray = sidecar(tmp.path(), "iVBORw0KGgostray");
    std::fs::write(&stray, "iVBORw0KGgostray").unwrap();

    messages.push(with_image("third", "image/png", "iVBORw0KGgothird"));
    store.save(&session(messages.clone())).await.unwrap();
    assert!(stray.is_file(), "an append removes nothing");
    assert!(sidecar(tmp.path(), "iVBORw0KGgothird").is_file());

    store.save(&session(messages.split_off(2))).await.unwrap();
    assert!(!stray.exists(), "a compaction collects it");
    assert!(sidecar(tmp.path(), "iVBORw0KGgothird").is_file());
}
