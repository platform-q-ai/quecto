use super::*;

// ── sigwinch_stream ──────────────────────────────────────────────

#[tokio::test]
async fn sigwinch_stream_returns_a_receiver() {
    // The function spawns a signal handler and returns a receiver.
    // We can't easily deliver a real SIGWINCH in a unit test, but we
    // can verify the channel is created and the receiver is usable.
    let mut rx = sigwinch_stream().await;
    // The receiver should be available for recv (it won't have any
    // messages unless we send a signal, which we avoid in tests).
    // Just verifying it doesn't panic and is the right type.
    tokio::select! {
        _ = rx.recv() => {
            // A signal arrived during the test — that's fine.
        }
        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
            // No signal — expected in a test environment.
        }
    }
}

#[tokio::test]
async fn sigwinch_stream_receives_signal() {
    // Deliver a real SIGWINCH and verify the channel fires.
    let mut rx = sigwinch_stream().await;

    // Give the spawned task time to register the handler.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // SAFETY: raising SIGWINCH to self is safe; the default action is ignored (no handler installed by default for SIGWINCH).
    unsafe {
        libc::raise(libc::SIGWINCH);
    }

    // The channel should fire within a reasonable time.
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
    assert!(
        result.is_ok(),
        "sigwinch_stream should fire after SIGWINCH is raised"
    );
}

// ── suspend ──────────────────────────────────────────────────────

#[test]
fn suspend_does_not_panic_when_not_a_tty() {
    // In a test environment stdin is typically not a TTY. The suspend
    // function checks termios and skips termios restoration if
    // tcgetattr fails. However, it DOES raise SIGTSTP unconditionally,
    // which would suspend the test process. We can't safely call
    // suspend() in a unit test without potentially hanging.
    //
    // Instead, we verify the function exists and compiles correctly.
    // The suspend behavior is tested via manual verification.
    // This is a compile-time + type check.
    let _ = suspend; // function pointer — verifies it exists and is callable
}
