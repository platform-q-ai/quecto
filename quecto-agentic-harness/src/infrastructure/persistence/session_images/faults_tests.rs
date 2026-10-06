//! Review round 2 (#2424), through the real file store: a fault on the
//! sidecar directory never blocks a save (the image is named, and back once
//! the fault is fixed); a load hashes no image its sidecar read verified;
//! the bounded read of another agent's transcript names its images.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use quecto_image::{ImageMime, samples};
use tempfile::TempDir;

use crate::application::sessions::ports::SessionStore;
use crate::domain::conversation::stored_images::user_image_types;
use crate::domain::message::{Message, UserImageBlock};
use crate::domain::sessions::entities::session::Session;
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::domain::tool::ImageBlock;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::persistence::session_store::session_store_records::session_store_bounded::UntrustedRead;

fn id() -> SessionIdentity {
    SessionIdentity::from_persisted_key("cli:faults")
}

fn store(dir: &Path) -> FileSessionStore {
    crate::composition::sessions::build_file_session_store(dir)
}

fn images_dir(dir: &Path) -> PathBuf {
    FlatSessionLayout::new(dir).image_dir(&id())
}

fn png() -> String {
    samples::encode(&samples::png(2, 3))
}

fn with_photo(text: &str) -> Message {
    let photo = UserImageBlock::restore(ImageMime::Png, png()).expect("admitted");
    Message::user(text).with_user_images(vec![photo])
}

fn with_screenshot() -> Message {
    let mut result = Message::tool("call-1", "Read image file");
    result.image_blocks = vec![ImageBlock::unchecked_for_tests(
        quecto_image::ImageMime::Jpeg,
        samples::encode(&samples::jpeg(2, 2)),
    )];
    result
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

#[tokio::test]
async fn an_unreadable_image_directory_never_blocks_a_save() {
    let tmp = TempDir::new().unwrap();
    let files = store(tmp.path());
    let mut messages = vec![with_photo("look"), Message::assistant("a photo", vec![])];
    files.save(&session(messages.clone())).await.unwrap();
    let dir = images_dir(tmp.path());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&dir).is_ok() {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        println!("skipped: mode 000 does not deny this user (root?), so there is no fault");
        return;
    }

    // A text-only turn, and a turn with a new image, while the fault lasts.
    messages.push(Message::user("and now?"));
    files
        .save_clean_delta(&id(), &messages, 2, None)
        .await
        .expect("a text-only turn saves");
    messages.push(with_screenshot());
    files
        .save_clean_delta(&id(), &messages, 3, None)
        .await
        .expect("a turn with an image saves");

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let loaded = reload(tmp.path()).await;
    assert_eq!(loaded.messages.len(), 4);
    assert_eq!(
        loaded.messages[0].user_image_blocks[0].data(),
        png(),
        "back once fixed"
    );
    // The image written during the fault is named, unloaded, until a save stores it.
    assert_eq!(loaded.messages[3].unloaded_images.len(), 1);
    files.save(&session(messages)).await.unwrap();
    assert_eq!(reload(tmp.path()).await.messages[3].image_blocks.len(), 1);
}

#[tokio::test]
async fn a_file_where_the_image_directory_should_be_never_blocks_a_save() {
    let tmp = TempDir::new().unwrap();
    let dir = images_dir(tmp.path());
    std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
    std::fs::write(&dir, "not a directory").unwrap();
    let files = store(tmp.path());
    let messages = vec![with_photo("look"), Message::assistant("a photo", vec![])];
    files
        .save(&session(messages.clone()))
        .await
        .expect("the record is written");
    assert_eq!(
        reload(tmp.path()).await.messages[0].unloaded_images.len(),
        1
    );

    std::fs::remove_file(&dir).unwrap();
    files.save(&session(messages)).await.unwrap();
    assert_eq!(
        reload(tmp.path()).await.messages[0].user_image_blocks[0].data(),
        png(),
        "the next save stores it"
    );
}

#[tokio::test]
async fn a_load_hashes_no_image_its_sidecar_read_verified() {
    let tmp = TempDir::new().unwrap();
    store(tmp.path())
        .save(&session(vec![
            with_photo("look"),
            with_screenshot(),
            Message::assistant("seen", vec![]),
        ]))
        .await
        .unwrap();
    let loaded = reload(tmp.path()).await;
    store(tmp.path()).save(&loaded).await.unwrap();
    assert_eq!(
        loaded.messages[0].user_image_blocks[0].digest_builds_for_tests(),
        0
    );
    assert_eq!(
        loaded.messages[1].image_blocks[0].digest_builds_for_tests(),
        0
    );
}

#[tokio::test]
async fn the_bounded_read_of_a_transcript_names_its_images() {
    let tmp = TempDir::new().unwrap();
    let files = store(tmp.path());
    let mut messages = vec![with_photo("look"), Message::assistant("a photo", vec![])];
    files.save(&session(messages.clone())).await.unwrap();
    messages.push(with_photo("again"));
    files
        .save_clean_delta(&id(), &messages, 2, None)
        .await
        .unwrap();
    for bound in [1 << 20, 400] {
        let read = files.load_untrusted(&id(), bound, None).await.unwrap();
        let UntrustedRead::Read { messages, .. } = read else {
            panic!("read");
        };
        let photos: Vec<&Message> = messages.iter().filter(|m| m.content != "a photo").collect();
        assert!(!photos.is_empty(), "{bound}");
        for photo in photos {
            assert!(photo.user_image_blocks.is_empty(), "no sidecar is read");
            assert_eq!(
                user_image_types(photo),
                ["image/png"],
                "but the image counts"
            );
        }
    }
}
