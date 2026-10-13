//! What a test's code writes to `tracing`, captured without a per-thread
//! subscriber (epic #2481 T3 review round 3).
//!
//! A scoped subscriber (`tracing::subscriber::with_default`) races
//! tracing-core's process-wide per-callsite interest cache: another thread
//! reaching a callsite with no subscriber of its own can cache it as
//! "never", and the scoped subscriber then sees nothing. So [`traced`]
//! installs one global subscriber, once, interested in every level, whose
//! writer appends only to the calling thread's armed buffer: a callsite is
//! then always of interest, and each test reads only what its own thread
//! wrote. Where another global subscriber was installed first (the lib
//! test binary's agent tests install the redacting one), it falls back to
//! a scoped subscriber and rebuilds the interest cache once that
//! subscriber is registered, which closes the window the cache left open.
use std::cell::RefCell;
use std::sync::OnceLock;

use tracing::{Level, Metadata};
use tracing_subscriber::fmt::MakeWriter;

thread_local! {
    /// The calling thread's capture: the most verbose level it keeps, and
    /// what it has kept.
    static ARMED: RefCell<Option<(Level, Vec<u8>)>> = const { RefCell::new(None) };
}

/// Appends to the calling thread's armed buffer, when it keeps the level.
struct ThreadWriter {
    keep: bool,
}

impl std::io::Write for ThreadWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.keep {
            ARMED.with(|armed| {
                if let Some((_, kept)) = armed.borrow_mut().as_mut() {
                    kept.extend_from_slice(bytes);
                }
            });
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The global subscriber's writers.
struct Capture;

impl<'a> MakeWriter<'a> for Capture {
    type Writer = ThreadWriter;

    fn make_writer(&'a self) -> ThreadWriter {
        ThreadWriter {
            keep: ARMED.with(|armed| armed.borrow().is_some()),
        }
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> ThreadWriter {
        let keep = ARMED.with(|armed| {
            armed
                .borrow()
                .as_ref()
                .is_some_and(|(most, _)| meta.level() <= most)
        });
        ThreadWriter { keep }
    }
}

fn subscriber() -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::fmt()
        .with_writer(Capture)
        .with_ansi(false)
        .with_max_level(Level::TRACE)
        .finish()
}

/// Whether the capture is the process's global subscriber.
fn global() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| tracing::subscriber::set_global_default(subscriber()).is_ok())
}

/// Disarms the thread's capture however `run` ends.
struct Armed;

impl Drop for Armed {
    fn drop(&mut self) {
        ARMED.with(|armed| armed.borrow_mut().take());
    }
}

/// What `run` writes to `tracing` on this thread, at `most` and every
/// more severe level.
pub fn traced(most: Level, run: impl FnOnce()) -> String {
    ARMED.with(|armed| *armed.borrow_mut() = Some((most, Vec::new())));
    let armed = Armed;
    match global() {
        true => run(),
        false => tracing::subscriber::with_default(subscriber(), || {
            tracing::callsite::rebuild_interest_cache();
            run();
        }),
    }
    let kept = ARMED.with(|armed| armed.borrow_mut().take().map(|(_, kept)| kept));
    drop(armed);
    String::from_utf8(kept.unwrap_or_default()).expect("tracing writes UTF-8")
}

#[cfg(test)]
#[path = "tracing_capture_tests.rs"]
mod tests;
