//! What a failed web_fetch request or body read says (#2209).
//!
//! reqwest's own message is only its top line ("error sending request for
//! url (...)"); the cause (a refused connection, a failed DNS lookup, a
//! rejected certificate) is in its `source()` chain. The chain is walked
//! and its distinct messages joined, led by a short plain reason when a
//! cause is one of a known few and by the redirect hop when the failure
//! was on one. URLs are named without their userinfo, query or fragment,
//! each at most [`MAX_URL_BYTES`] long, and the URL asked for comes after
//! the causes, so no URL can push them out of the bounded detail.
//!
//! A redirect hop's URL is the server's choice and may carry a token in
//! its path (`/auth/callback/<token>`): it is named by its first path
//! segment only, the rest standing as [`HIDDEN_PATH`] (#2248 round 2). The
//! URL asked for is the agent's own input and keeps its whole path, within
//! the same bound, so that the agent sees which of its URLs failed.
use std::error::Error;
use std::io::ErrorKind;

/// The longest detail a failure carries, in bytes, any hint included.
pub(super) const MAX_DETAIL_BYTES: usize = 512;

/// The longest URL a detail names, in bytes (#2248 review): room is always
/// left for the causes.
pub(super) const MAX_URL_BYTES: usize = 128;

/// Marks a detail or a URL cut at its bound.
const CUT_MARK: char = '…';

/// Stands for a URL's query, which is never shown: a server's redirect
/// commonly carries tokens there (#2248 review).
const HIDDEN_QUERY: &str = "?…";

/// Stands for a hop's path past its first segment, never shown (#2248
/// round 2).
const HIDDEN_PATH: &str = "/…";

/// Plain reasons for the I/O error kinds a cause may carry: only these are
/// named, every other cause speaks for itself in the chain.
const IO_REASONS: &[(ErrorKind, &str)] = &[
    (ErrorKind::ConnectionRefused, "connection refused"),
    (ErrorKind::ConnectionReset, "connection reset by the server"),
    (ErrorKind::ConnectionAborted, "connection aborted"),
    (ErrorKind::TimedOut, "timed out"),
    (ErrorKind::HostUnreachable, "host unreachable"),
    (ErrorKind::NetworkUnreachable, "network unreachable"),
];

/// Plain reasons for causes known by their message: hyper-util's resolver
/// failure and rustls's handshake failures.
pub(super) const MESSAGE_REASONS: &[(&str, &str)] = &[
    ("dns error", "DNS lookup failed"),
    ("invalid peer certificate", "TLS certificate not accepted"),
    ("received fatal alert", "TLS handshake failed"),
    ("received corrupt message", "TLS handshake failed"),
];

/// What was being fetched when a request or read failed.
#[derive(Clone, Copy, Debug)]
pub(super) enum Attempt<'a> {
    /// The URL asked for, before any redirect was followed.
    Requested(&'a url::Url),
    /// A redirect hop, after at least one redirect was followed, whatever
    /// its URL (a loop may lead back to the one asked for).
    Hop(&'a url::Url),
}

impl<'a> Attempt<'a> {
    /// `url`, a hop when `follows` redirects were followed before it.
    pub(super) fn after(follows: usize, url: &'a url::Url) -> Self {
        match follows {
            0 => Self::Requested(url),
            _ => Self::Hop(url),
        }
    }
}

/// The detail of a failed request or read: `error` (whose own message
/// names no URL) and its causes, a plain reason when one is known, and the
/// URL of the `attempt`: its hop leads, the URL asked for comes last.
pub(super) fn describe(error: &(dyn Error + 'static), attempt: Attempt<'_>) -> String {
    let reason = reason(error);
    let lead = match (reason, attempt) {
        (Some(reason), Attempt::Hop(hop)) => Some(format!(
            "{reason} on the redirect hop to {}",
            shown_hop(hop)
        )),
        (Some(reason), Attempt::Requested(_)) => Some(reason.to_owned()),
        (None, Attempt::Hop(hop)) => {
            Some(format!("failed on the redirect hop to {}", shown_hop(hop)))
        }
        (None, Attempt::Requested(_)) => None,
    };
    let mut messages = causes(error).map(ToString::to_string);
    let mut joined = messages.next().unwrap_or_default();
    for message in messages {
        // reqwest and hyper repeat a cause in their own message: said once
        // (an empty one is always contained, so it adds nothing).
        if !joined.contains(&message) {
            joined.push_str(": ");
            joined.push_str(&message);
        }
    }
    // The hop is named in the lead; the URL asked for follows the causes.
    if let Attempt::Requested(requested) = attempt {
        joined.push_str(&format!(", for url ({})", shown(requested)));
    }
    let detail = match lead {
        Some(lead) => format!("{lead}: {joined}"),
        None => joined,
    };
    bounded(detail)
}

/// `error` and every cause after it.
fn causes<'a>(error: &'a (dyn Error + 'static)) -> impl Iterator<Item = &'a (dyn Error + 'static)> {
    std::iter::successors(Some(error), |&current| current.source())
}

/// The plain reason of the first cause that has one.
fn reason(error: &(dyn Error + 'static)) -> Option<&'static str> {
    causes(error).find_map(|cause| {
        let text = cause.to_string();
        let by_message = MESSAGE_REASONS
            .iter()
            .find(|(marker, _)| text.contains(marker))
            .map(|(_, reason)| *reason);
        let by_kind = || {
            let kind = cause.downcast_ref::<std::io::Error>()?.kind();
            IO_REASONS
                .iter()
                .find(|(known, _)| *known == kind)
                .map(|(_, reason)| *reason)
        };
        by_message.or_else(by_kind)
    })
}

/// The URL asked for as a failure names it (#2248 review): its scheme,
/// host, port and path only, the query standing as [`HIDDEN_QUERY`] when
/// there is one, at most [`MAX_URL_BYTES`] long.
pub(super) fn shown(url: &url::Url) -> String {
    named(url, None)
}

/// A redirect hop as a failure names it (#2248 round 2): as [`shown`], but
/// its path past the first segment stands as [`HIDDEN_PATH`].
pub(super) fn shown_hop(url: &url::Url) -> String {
    let first_segment = url
        .path()
        .strip_prefix('/')
        .and_then(|path| path.split_once('/'))
        .and_then(|(first, rest)| match rest {
            "" => None,
            _ => Some(first),
        });
    named(url, first_segment)
}

/// `url` without its userinfo, query or fragment, its path cut to
/// `first_segment` and [`HIDDEN_PATH`] when there is one, and the query
/// standing as [`HIDDEN_QUERY`], at most [`MAX_URL_BYTES`] long.
fn named(url: &url::Url, first_segment: Option<&str>) -> String {
    let has_query = url.query().is_some_and(|query| !query.is_empty());
    let mut kept = url.clone();
    // None can fail on an http(s) URL, the only kind fetched.
    let _ = kept.set_username("");
    let _ = kept.set_password(None);
    kept.set_query(None);
    kept.set_fragment(None);
    if let Some(first) = first_segment {
        kept.set_path(&format!("/{first}"));
    }
    let mut text = kept.to_string();
    if first_segment.is_some() {
        text.push_str(HIDDEN_PATH);
    }
    if has_query {
        text.push_str(HIDDEN_QUERY);
    }
    cut_to(text, MAX_URL_BYTES)
}

/// `detail`, cut on a character boundary to at most [`MAX_DETAIL_BYTES`]
/// with [`CUT_MARK`] when it is longer.
fn bounded(detail: String) -> String {
    cut_to(detail, MAX_DETAIL_BYTES)
}

/// `message` followed by `hint`, the message cut to leave room for the
/// hint so that the whole stays within [`MAX_DETAIL_BYTES`] (#2248 review).
pub(super) fn with_hint(message: &str, hint: &str) -> String {
    assert!(
        hint.len() < MAX_DETAIL_BYTES / 2,
        "a hint crowding out its detail"
    );
    let room = MAX_DETAIL_BYTES - hint.len() - 1;
    let hinted = format!("{} {hint}", cut_to(message.to_owned(), room));
    assert!(
        hinted.len() <= MAX_DETAIL_BYTES,
        "a hinted detail over its bound"
    );
    hinted
}

/// `text`, cut on a character boundary to at most `max` bytes with
/// [`CUT_MARK`] when it is longer.
fn cut_to(mut text: String, max: usize) -> String {
    assert!(
        max >= CUT_MARK.len_utf8(),
        "a bound with no room for its mark"
    );
    if text.len() > max {
        let mut end = max - CUT_MARK.len_utf8();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push(CUT_MARK);
    }
    assert!(text.len() <= max, "text over its bound");
    text
}

#[cfg(test)]
#[path = "web_fetch_failure_tests.rs"]
mod tests;
