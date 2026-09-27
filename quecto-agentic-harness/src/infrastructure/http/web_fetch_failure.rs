//! What a failed web_fetch request or body read says (#2209).
//!
//! reqwest's own message is only its top line ("error sending request for
//! url (...)"); the cause (a refused connection, a failed DNS lookup, a
//! rejected certificate) is in its `source()` chain. The chain is walked
//! and its distinct messages joined, led by a short plain reason when a
//! cause is one of a known few and by the redirect hop when the failure
//! was on one. URLs are named without their userinfo, and the whole detail
//! is bounded.
use std::error::Error;
use std::io::ErrorKind;

/// The longest detail a failure carries, in bytes.
pub(super) const MAX_DETAIL_BYTES: usize = 512;

/// Marks a detail cut at [`MAX_DETAIL_BYTES`].
const CUT_MARK: char = '…';

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
const MESSAGE_REASONS: &[(&str, &str)] = &[
    ("dns error", "DNS lookup failed"),
    ("invalid peer certificate", "TLS certificate not accepted"),
    ("received fatal alert", "TLS handshake failed"),
    ("received corrupt message", "TLS handshake failed"),
];

/// The detail of a failed request or read: `error` (whose own message
/// names no URL) and its causes, the attempt's URL, a plain reason when
/// one is known, and the redirect hop when `attempted` is not `requested`.
pub(super) fn describe(
    error: &(dyn Error + 'static),
    requested: &url::Url,
    attempted: &url::Url,
) -> String {
    let hop = is_redirect_hop(requested, attempted).then(|| shown(attempted));
    let lead = match (reason(error), &hop) {
        (Some(reason), Some(hop)) => Some(format!("{reason} on the redirect hop to {hop}")),
        (Some(reason), None) => Some(reason.to_owned()),
        (None, Some(hop)) => Some(format!("failed on the redirect hop to {hop}")),
        (None, None) => None,
    };
    let mut messages = causes(error).map(ToString::to_string);
    let top = messages.next().unwrap_or_default();
    // The hop is named in the lead; otherwise the top line names the URL.
    let mut joined = match hop {
        Some(_) => top,
        None => format!("{top} for url ({})", shown(requested)),
    };
    for message in messages {
        // reqwest and hyper repeat a cause in their own message: said once
        // (an empty one is always contained, so it adds nothing).
        if !joined.contains(&message) {
            joined.push_str(": ");
            joined.push_str(&message);
        }
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

/// Whether the attempt was on a redirect hop: its URL is another than the
/// one asked for, credentials and fragment aside.
fn is_redirect_hop(requested: &url::Url, attempted: &url::Url) -> bool {
    let mut requested = without_userinfo(requested);
    let mut attempted = without_userinfo(attempted);
    requested.set_fragment(None);
    attempted.set_fragment(None);
    requested != attempted
}

/// A URL as a failure names it: without userinfo.
fn shown(url: &url::Url) -> String {
    without_userinfo(url).to_string()
}

fn without_userinfo(url: &url::Url) -> url::Url {
    let mut clean = url.clone();
    // Neither can fail on an http(s) URL, the only kind fetched.
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean
}

/// `detail`, cut on a character boundary to at most [`MAX_DETAIL_BYTES`]
/// with [`CUT_MARK`] when it is longer.
fn bounded(mut detail: String) -> String {
    if detail.len() > MAX_DETAIL_BYTES {
        let mut end = MAX_DETAIL_BYTES - CUT_MARK.len_utf8();
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
        detail.push(CUT_MARK);
    }
    assert!(detail.len() <= MAX_DETAIL_BYTES, "a detail over its bound");
    detail
}

#[cfg(test)]
#[path = "web_fetch_failure_tests.rs"]
mod tests;
