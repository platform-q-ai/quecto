//! How a sub-agent ended, as its parent tells it (#2192): the exit status
//! the parent observed and the crash record the child left. Everything a
//! crash record says was written by the child — and any container can write
//! one under another child's key — so it is only believed when an observed
//! end fits a fatal panic of that very process. With no end observed (a
//! container child, a merged descendant) the end stays unknown and the
//! record is only quoted. Whatever of it reaches the parent is escaped,
//! capped and labelled as the child's unverified words — never text the
//! parent could take as an instruction.
use crate::domain::crash_record::{CrashRecord, PanicReport};

/// The signal a fatal panic ends the process with (`SIGABRT`).
pub const ABORT_SIGNAL: i32 = 6;

/// The status a test-support build ends a fatal panic with instead of an
/// abort (it must not dump core); only such a build believes it.
pub const TEST_FATAL_EXIT_CODE: i32 = 134;

/// The most of a child's panic message the parent is shown, in bytes.
pub const MAX_SHOWN_PANIC_BYTES: usize = 512;

/// The most of a child-supplied name (a tool, a location) shown, in bytes.
pub const MAX_SHOWN_NAME_BYTES: usize = 128;

/// Where a sub-agent's row came from, as far as this harness can vouch for
/// it (#2192 review). Only `Launched` is vouched for: its uuid, session
/// and pid are the ones this harness gave the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildOrigin {
    /// This harness launched the child itself.
    Launched,
    /// A child reported it as one of its descendants: every field of it is
    /// that child's word.
    Reported,
    /// A row made neither by a launch nor from a report (a fixture, a
    /// hand-built row): nothing about it is vouched for here. A saved
    /// roster's rows never re-enter the live registry after a restart, so
    /// a child that ended before one is not found here at all.
    Unverified,
}

impl ChildOrigin {
    /// Whether a launch under the same key takes the row over: only a row
    /// a child reported does (#2192 review) — this harness's own launch is
    /// authoritative over another agent's word.
    pub fn yields_to_a_launch(self) -> bool {
        matches!(self, Self::Reported)
    }
}

/// How a sub-agent ended, as far as its parent can tell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChildEnd {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// The child's pid, when this harness launched it and knows it.
    pub pid: Option<u32>,
    pub crash: Option<CrashRecord>,
}

/// What kind of end a [`ChildEnd`] was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndKind {
    /// It exited with status 0, and no believed crash record says otherwise.
    Clean,
    /// A believed crash, a signal, or a failure status.
    Abnormal,
    /// Nothing beyond "it ended" was observed.
    Unknown,
}

impl ChildEnd {
    /// The crash record, when it is believed: an observed end fits a fatal
    /// panic (an abort, or the test-support exit) and the record names the
    /// child's pid — known because this harness launched it. "Believed" is
    /// that match, no more: the record is not authenticated, and within the
    /// same-uid trust model anyone who can write the crash directory can
    /// forge one naming the right pid (#2192 review). With no status
    /// observed nothing ties the record to this child's end, and with no
    /// pid known nothing ties it to this child's process, so neither is
    /// believed.
    pub fn believed_crash(&self) -> Option<&CrashRecord> {
        let crash = self.crash.as_ref()?;
        let test_exit = cfg!(any(test, feature = "test-support"));
        let fatal_end = match (self.exit_code, self.signal) {
            (None, Some(ABORT_SIGNAL)) => true,
            (Some(TEST_FATAL_EXIT_CODE), None) => test_exit,
            (Some(_), _) | (None, Some(_)) | (None, None) => false,
        };
        let own_process = self.pid == Some(crash.pid);
        (fatal_end && own_process).then_some(crash)
    }

    /// Why a crash record that was left is not believed, if it is not.
    fn disbelief(&self) -> Option<&'static str> {
        match (&self.crash, self.believed_crash(), self.pid) {
            (Some(_), None, Some(_)) => {
                Some("; a crash record it left does not fit that end and was not believed")
            }
            (Some(_), None, None) => {
                Some("; a crash record it left cannot be tied to its process and was not believed")
            }
            (Some(_), Some(_), _) | (None, _, _) => None,
        }
    }

    /// A crash record left with no end observed to check it against,
    /// quoted as the child's words — its attribution included.
    fn unchecked_record(&self) -> Option<String> {
        let crash = match (self.exit_code, self.signal) {
            (None, None) => self.crash.as_ref()?,
            (Some(_), _) | (None, Some(_)) => return None,
        };
        let said = match attribution(crash) {
            Some(attribution) => format!("{attribution}: {}", panic_text(crash)),
            None => panic_text(crash),
        };
        Some(format!(
            "ended; no exit status was observed, so a crash record it left cannot be checked \
             against its end. The record says (child-supplied, unverified): {said}"
        ))
    }

    pub fn kind(&self) -> EndKind {
        match (self.believed_crash(), self.signal, self.exit_code) {
            (Some(_), _, _) | (None, Some(_), _) => EndKind::Abnormal,
            (None, None, Some(0)) => EndKind::Clean,
            (None, None, Some(_)) => EndKind::Abnormal,
            (None, None, None) => EndKind::Unknown,
        }
    }

    /// The end, to follow the sub-agent's name, in the one wording every
    /// view of it uses: "ended normally (exit code 0)", "ended unexpectedly
    /// (signal 6) while tool call 'edit' was running (child-supplied):
    /// panicked; the child's recorded panic message: \"…\" (child-supplied,
    /// unverified)", "ended; no exit status or crash record was observed". A
    /// panic is attributed to a call only when it came from that call's own
    /// scope ("during tool call 'edit'"), and the attribution is labelled as
    /// the child's word even when its record is believed. A crash record
    /// that does not fit the end, or cannot be tied to the child's process,
    /// is named as such and not believed.
    pub fn reason(&self) -> String {
        let crash = self.believed_crash();
        let mut parts: Vec<String> = Vec::new();
        parts.extend(status(self.exit_code, self.signal));
        // Believed or not, which call it names is the child's own word.
        parts.extend(
            crash
                .and_then(attribution)
                .map(|attribution| format!("{attribution} (child-supplied)")),
        );
        let described = parts.join(" ");
        let details = match (described.is_empty(), crash.map(panic_text)) {
            (true, None) => String::new(),
            (true, Some(panic)) => format!(" {panic}"),
            (false, None) => format!(" {described}"),
            (false, Some(panic)) => format!(" {described}: {panic}"),
        };
        let note = self.disbelief().unwrap_or("");
        match self.kind() {
            EndKind::Clean => format!("ended normally{details}{note}"),
            EndKind::Abnormal => format!("ended unexpectedly{details}{note}"),
            EndKind::Unknown => match self.unchecked_record() {
                Some(unchecked) => unchecked,
                None => format!("ended; no exit status or crash record was observed{note}"),
            },
        }
    }
}

/// `text` made safe to show a parent: every control character, quote and
/// backslash escaped, then cut to at most `max` bytes (marked with "…" when
/// cut) — only between whole escapes, never inside one (`\u{1…`).
pub fn shown(text: &str, max: usize) -> String {
    let escaped: String = text.escape_debug().collect();
    if escaped.len() <= max {
        return escaped;
    }
    let mut cut = String::with_capacity(max + "…".len());
    for piece in text.chars().map(char::escape_debug) {
        let piece = piece.to_string();
        if cut.len() + piece.len() > max {
            break;
        }
        cut.push_str(&piece);
    }
    cut.push('…');
    cut
}

fn name(text: &str) -> String {
    shown(text, MAX_SHOWN_NAME_BYTES)
}

/// Which call a crash is attributed to, or which were running.
fn attribution(crash: &CrashRecord) -> Option<String> {
    let quoted = |calls: &[String]| {
        calls
            .iter()
            .map(|tool| format!("'{}'", name(tool)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (&crash.call, crash.running.as_slice()) {
        (Some(tool), _) => Some(format!("during tool call '{}'", name(tool))),
        (None, []) => None,
        (None, [tool]) => Some(format!("while tool call '{}' was running", name(tool))),
        (None, calls) => Some(format!("while tool calls {} were running", quoted(calls))),
    }
}

fn status(exit_code: Option<i32>, signal: Option<i32>) -> Option<String> {
    match (exit_code, signal) {
        (Some(code), Some(signal)) => Some(format!("(exit code {code} / signal {signal})")),
        (Some(code), None) => Some(format!("(exit code {code})")),
        (None, Some(signal)) => Some(format!("(signal {signal})")),
        (None, None) => None,
    }
}

/// A panic report as data: the child's words, escaped, capped and quoted.
fn report_text(report: &PanicReport) -> String {
    let at = report
        .location
        .as_deref()
        .map(|location| format!(" at \"{}\"", name(location)))
        .unwrap_or_default();
    format!("\"{}\"{at}", shown(&report.message, MAX_SHOWN_PANIC_BYTES))
}

fn panic_text(crash: &CrashRecord) -> String {
    let kind = match crash.provisional {
        true => "panicked, and it ended before the call contained the panic",
        false => "panicked",
    };
    let earlier = crash
        .earlier
        .as_ref()
        .map(|earlier| {
            format!(
                ", struck while the call's own panic {} was unwinding",
                report_text(earlier)
            )
        })
        .unwrap_or_default();
    format!(
        "{kind}; the child's recorded panic message: {}{earlier} (child-supplied, unverified)",
        report_text(&crash.panic)
    )
}

#[cfg(test)]
#[path = "child_end_tests.rs"]
mod tests;
