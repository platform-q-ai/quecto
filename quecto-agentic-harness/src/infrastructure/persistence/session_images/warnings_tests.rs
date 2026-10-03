//! Review round 3 (#2424): a sidecar write failure is warned about once per
//! image of each session, again after a success and a new failure, and once
//! more when its session is left with the image never stored.
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use quecto_image::{ImageMime, samples};

use super::{ImageSidecarStore, SessionImages, SidecarFuture, SidecarRead};
use crate::domain::conversation::stored_images::ImageRef;
use crate::domain::error::DomainError;
use crate::domain::message::{Message, UserImageBlock};
use crate::domain::session_identity::SessionIdentity;

/// Sidecars whose writes fail while `failing` is set.
#[derive(Default)]
struct Flaky {
    failing: AtomicBool,
}

impl ImageSidecarStore for Flaky {
    fn put<'a>(
        &'a self,
        _identity: &'a SessionIdentity,
        _reference: &'a ImageRef,
        _text: &'a str,
    ) -> SidecarFuture<'a, ()> {
        let failing = self.failing.load(Ordering::Relaxed);
        Box::pin(async move {
            match failing {
                true => Err(DomainError::Session("EACCES (injected)".into())),
                false => Ok(()),
            }
        })
    }

    fn get<'a>(&'a self, _: &'a SessionIdentity, _: &'a str) -> SidecarFuture<'a, SidecarRead> {
        Box::pin(async { Ok(SidecarRead::Missing) })
    }

    fn retain_only<'a>(
        &'a self,
        _: &'a SessionIdentity,
        _: &'a BTreeSet<String>,
    ) -> SidecarFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn remove_all<'a>(&'a self, _: &'a SessionIdentity) -> SidecarFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn session(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

fn with_photo() -> Vec<Message> {
    let photo = UserImageBlock::restore(ImageMime::Png, samples::encode(&samples::png(2, 3)));
    vec![Message::user("look").with_user_images(vec![photo.expect("admitted")])]
}

#[tokio::test]
async fn a_failure_is_warned_once_per_image_and_again_after_a_success() {
    let sidecars = Arc::new(Flaky::default());
    let images = SessionImages::over(sidecars.clone());
    let messages = with_photo();
    sidecars.failing.store(true, Ordering::Relaxed);
    images.store(&session("cli:a"), &messages).await;
    images.store(&session("cli:a"), &messages).await;
    assert_eq!(
        images.warnings_for_tests(),
        1,
        "once while it keeps failing"
    );

    sidecars.failing.store(false, Ordering::Relaxed);
    images.store(&session("cli:a"), &messages).await;
    sidecars.failing.store(true, Ordering::Relaxed);
    images.store(&session("cli:a"), &messages).await;
    assert_eq!(
        images.warnings_for_tests(),
        2,
        "a new failure after a success"
    );
}

#[tokio::test]
async fn the_same_image_failing_in_two_sessions_is_warned_for_each() {
    let sidecars = Arc::new(Flaky::default());
    let images = SessionImages::over(sidecars.clone());
    sidecars.failing.store(true, Ordering::Relaxed);
    let messages = with_photo();
    images.store(&session("cli:a"), &messages).await;
    images.store(&session("cli:b"), &messages).await;
    assert_eq!(images.warnings_for_tests(), 2);

    // Session b's image is stored; session a's still fails.
    sidecars.failing.store(false, Ordering::Relaxed);
    images.store(&session("cli:b"), &messages).await;
    images.leave(&session("cli:b"));
    assert_eq!(images.warnings_for_tests(), 2, "b leaves with nothing lost");
    images.leave(&session("cli:a"));
    assert_eq!(
        images.warnings_for_tests(),
        3,
        "a leaves with its image lost: said once"
    );
    images.leave(&session("cli:a"));
    assert_eq!(images.warnings_for_tests(), 3);
}
