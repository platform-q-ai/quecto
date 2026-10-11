//! What `git push --porcelain` reported for the board's ref, and the
//! report a push that did not land carries into its outcome.
use super::git::quoted;
use super::store::BOARD_REF;
use std::fmt;

/// What a push that did not report our ref updated said: our ref's
/// rejection, if it reported one, and git's own message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    pub rejected: Option<String>,
    pub said: String,
}

impl fmt::Display for PushReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.rejected, self.said.is_empty()) {
            (Some(rejected), true) => f.write_str(rejected),
            (Some(rejected), false) => write!(f, "{rejected}; git said: {}", self.said),
            (None, true) => f.write_str("no status for the board"),
            (None, false) => f.write_str(&self.said),
        }
    }
}

/// Our ref's line in `git push --porcelain`: `<flag>\t<from>:<to>\t<summary>`.
#[derive(Debug, PartialEq, Eq)]
pub enum RefStatus {
    /// ` ` fast-forward, `+` forced, `*` new, `=` already there.
    Updated,
    /// `!` rejected, with the summary's reason.
    Rejected(String),
}

pub fn ref_status(stdout: &[u8]) -> Option<RefStatus> {
    let text = String::from_utf8_lossy(stdout);
    let ours = format!(":{BOARD_REF}");
    text.lines().find_map(|row| {
        let mut fields = row.splitn(3, '\t');
        let (flag, refs, summary) = (fields.next()?, fields.next()?, fields.next().unwrap_or(""));
        if !refs.ends_with(&ours) {
            return None;
        }
        match flag {
            " " | "+" | "*" | "=" => Some(RefStatus::Updated),
            "!" => Some(RefStatus::Rejected(quoted(summary.as_bytes()))),
            _ => None,
        }
    })
}

/// What the remote's own rejection of a push says, by its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// A hook, a protected branch or a rule declined it: refused.
    Declined,
    /// It lost a race on the ref: lost.
    Raced,
    /// Any other reason: whether it may still land is unknown.
    Unknown,
}

/// The reasons a remote gives when it declines a push: git's hooks
/// ("pre-receive hook declined", "hook declined"), GitHub's protected
/// branches ("protected branch hook declined") and rulesets ("push declined
/// due to repository rule violations").
const DECLINED: &[&str] = &["hook declined", "protected branch", "push declined"];
/// The reasons a remote gives when another update of the ref won: it found
/// another value than the lease's, or could not lock the ref because it is
/// at another value ("cannot lock ref '<ref>': is at <a> but expected
/// <b>"). Any other lock failure (a stale lock file, say) is unknown.
fn raced(reason: &str) -> bool {
    reason.contains("incorrect old value")
        || (reason.contains("cannot lock ref") && reason.contains("but expected"))
}

/// What a `[remote rejected] (<reason>)` summary says; `None` for any
/// other summary.
pub fn remote_rejection(summary: &str) -> Option<Rejection> {
    let reason = summary.strip_prefix("[remote rejected]")?;
    let said = |reasons: &[&str]| reasons.iter().any(|known| reason.contains(known));
    Some(match (said(DECLINED), raced(reason)) {
        (true, false) => Rejection::Declined,
        (false, true) => Rejection::Raced,
        _ => Rejection::Unknown,
    })
}
