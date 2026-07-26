use std::ops::{Deref, DerefMut};

use super::*;
use crate::shell::terminal::Terminal;
use tempfile::TempDir;
use tokio::io::AsyncReadExt;

pub(super) struct TestApp {
    app: App,
    pub(super) dir: TempDir,
}

impl Deref for TestApp {
    type Target = App;

    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.app
    }
}

pub(super) async fn test_app() -> TestApp {
    let dir = tempfile::Builder::new()
        .prefix("quecto-tui-app-events-test-")
        .tempdir()
        .expect("create unique app_events temp dir");
    let socket_path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap_or_else(|err| {
        panic!(
            "bind app_events test socket at {}: {err}",
            socket_path.display()
        )
    });
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    });
    let client = Client::connect(&socket_path).await.unwrap();
    TestApp {
        app: App::new(Terminal::new(), client),
        dir,
    }
}

#[tokio::test]
async fn app_events_test_apps_use_unique_self_cleaning_socket_dirs() {
    let first = test_app().await;
    let second = test_app().await;

    let first_path = first.dir.path().to_path_buf();
    let second_path = second.dir.path().to_path_buf();
    assert_ne!(first_path, second_path);
    assert!(first_path.join("agent.sock").exists());
    assert!(second_path.join("agent.sock").exists());

    drop(first);
    assert!(
        !first_path.exists(),
        "dropping TestApp must clean up its socket directory"
    );
    assert!(
        second_path.exists(),
        "a live TestApp must keep its socket directory available"
    );
}
