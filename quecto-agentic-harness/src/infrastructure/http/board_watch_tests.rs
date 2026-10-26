//! The watch's decoding (epic #2705 B10): the feed's schema, an allowlist.
use super::*;

fn event(id: &str, name: &str, data: &str) -> StreamEvent {
    StreamEvent {
        id: Some(id.into()),
        event: name.into(),
        data: data.into(),
    }
}

#[test]
fn each_kind_of_the_schema_is_decoded() {
    assert_eq!(
        decoded(&event("3", "epoch", r#"{"kind":"epoch","epoch":2}"#)),
        Some((3, FeedEvent::Epoch { epoch: 2 }))
    );
    assert_eq!(
        decoded(&event(
            "4",
            "revision",
            r#"{"kind":"revision","epoch":2,"revision":4}"#
        )),
        Some((
            4,
            FeedEvent::Revision(BoardRevision {
                epoch: 2,
                revision: 4
            })
        ))
    );
    assert_eq!(
        decoded(&event("4", "reset", r#"{"kind":"reset","after":4}"#)),
        Some((4, FeedEvent::Reset { after: 4 }))
    );
}

#[test]
fn anything_off_the_schema_is_refused() {
    for (id, name, data) in [
        // Text riding along is never taken.
        (
            "4",
            "revision",
            r#"{"kind":"revision","epoch":2,"revision":4,"title":"x"}"#,
        ),
        ("4", "revision", r#"{"kind":"revision","epoch":2}"#),
        (
            "5",
            "revision",
            r#"{"kind":"revision","epoch":2,"revision":4}"#,
        ),
        (
            "4",
            "epoch",
            r#"{"kind":"revision","epoch":2,"revision":4}"#,
        ),
        ("4", "message", r#"{"kind":"message","text":"x"}"#),
        ("x", "epoch", r#"{"kind":"epoch","epoch":2}"#),
        ("3", "epoch", r#"{"kind":"epoch","epoch":"2"}"#),
        ("3", "reset", r#"{"kind":"reset","after":2}"#),
        ("3", "epoch", "not json"),
    ] {
        assert_eq!(decoded(&event(id, name, data)), None, "{name} {data}");
    }
    let no_id = StreamEvent {
        id: None,
        ..event("3", "epoch", r#"{"kind":"epoch","epoch":2}"#)
    };
    assert_eq!(decoded(&no_id), None);
}

/// Review (#2757 round 3, L1): a pipe's write end has a reader while its
/// read end is open, and none once it is closed.
#[test]
fn a_pipe_has_a_reader_until_its_read_end_closes() {
    use std::os::fd::AsRawFd;
    let (reader, writer) = std::io::pipe().expect("a pipe");
    assert!(has_reader(writer.as_raw_fd()), "its reader is open");
    drop(reader);
    assert!(!has_reader(writer.as_raw_fd()), "its reader is gone");
}

/// Review (#2757 round 4): a terminal whose other side has gone (a pty's
/// slave once its master closes) polls `POLLHUP`: no reader.
#[test]
fn a_hung_up_terminal_has_no_reader() {
    use std::os::fd::{FromRawFd, OwnedFd};
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: two valid out-pointers; no name, termios or window size.
    let made = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    assert_eq!(made, 0, "a pty: {}", std::io::Error::last_os_error());
    // SAFETY: openpty gave both descriptors to this test alone.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    use std::os::fd::AsRawFd;
    assert!(has_reader(slave.as_raw_fd()), "its master is open");
    drop(master);
    assert!(!has_reader(slave.as_raw_fd()), "its master is gone");
}
