use super::tests::StubStore;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};

#[tokio::test]
async fn stub_store_default_session_store_methods_delegate_to_save() {
    let store = StubStore::with_loaded(Ok(None));
    let messages = vec![Message::user("delta")];

    store
        .save_delta("cli:delta", &messages, 0, None)
        .await
        .expect("default save_delta should call stub save");
    store
        .save_clean_delta("cli:clean", &messages, 0, None)
        .await
        .expect("default save_clean_delta should call default save_delta");
}

#[tokio::test]
async fn stub_store_inert_surface_methods_return_empty_or_false() {
    let store = StubStore::with_loaded(Ok(Some(Session::new("cli:loaded"))));

    assert!(!store.exists("cli:any").await.unwrap());
    assert!(store.list(None).await.unwrap().is_empty());
    store.save(&Session::new("cli:any")).await.unwrap();
}
