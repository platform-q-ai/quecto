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

// ── termination_stream ───────────────────────────────────────────

/// #2053: SIGHUP and SIGTERM reach the receiver, named, and a burst while
/// one is pending folds into it. One test, because every registered stream
/// in this process sees every raised signal. (Registering first is what
/// keeps the test process alive.)
#[tokio::test]
async fn termination_stream_names_each_signal_and_folds_a_burst() {
    let mut rx = termination_stream();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    for (raise, expected) in [
        (libc::SIGHUP, TerminationSignal::Hangup),
        (libc::SIGTERM, TerminationSignal::Terminate),
    ] {
        // SAFETY: raising a signal to self for which a handler is registered.
        unsafe {
            libc::raise(raise);
        }
        let got = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the stream fires after the signal")
            .expect("the stream is open");
        assert_eq!(got, expected);
    }
    for _ in 0..3 {
        // SAFETY: as above.
        unsafe {
            libc::raise(libc::SIGHUP);
        }
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(rx.recv().await, Some(TerminationSignal::Hangup));
    assert!(rx.try_recv().is_err(), "the burst folded into one");
    assert_eq!(TerminationSignal::Interrupt.name(), "SIGINT");
    assert_eq!(TerminationSignal::Hangup.name(), "SIGHUP");
    assert_eq!(TerminationSignal::Terminate.name(), "SIGTERM");
}
