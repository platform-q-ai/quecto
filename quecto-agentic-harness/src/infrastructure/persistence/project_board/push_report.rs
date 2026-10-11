//! What `git push --porcelain` reported for the board's ref, and the
//! report a push that did not land carries into its outcome.
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
            "!" => Some(RefStatus::Rejected(summary.to_string())),
            _ => None,
        }
    })
}
