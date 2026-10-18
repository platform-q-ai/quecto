//! The names crash records take (#2192 review), told apart exactly: a
//! session's clear and the startup sweep remove only entries whose names
//! have one of these shapes, never anything else found in the directory.
use super::super::filename::DIGEST_NAME_LEN;

/// What a crash directory entry's name says it is. `stem` is the session
/// key's digest ([`super::super::filename::digest_session_key`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecordName<'a> {
    /// `<stem>.crash`: a fatal record.
    Fatal { stem: &'a str },
    /// `<stem>.crash.provisional.<pid>.<scope>`, in decimal digits.
    Provisional { stem: &'a str, pid: u64, scope: u64 },
    /// `.<a fatal or provisional name>.<16 lowercase hex>.tmp`: the
    /// temporary file of a write that never reached its rename.
    Temporary { stem: &'a str },
}

impl<'a> RecordName<'a> {
    /// The shape `name` has, if it is one of a record's.
    pub(super) fn parse(name: &'a str) -> Option<Self> {
        match name.strip_prefix('.') {
            Some(hidden) => Self::temporary(hidden),
            None => Self::record(name),
        }
    }

    /// The digest of the session the name is of.
    pub(super) fn stem(&self) -> &'a str {
        match self {
            Self::Fatal { stem } | Self::Provisional { stem, .. } | Self::Temporary { stem } => {
                stem
            }
        }
    }

    fn record(name: &'a str) -> Option<Self> {
        let (stem, rest) = name.split_at_checked(DIGEST_NAME_LEN)?;
        match (is_lower_hex(stem), rest) {
            (true, ".crash") => Some(Self::Fatal { stem }),
            (true, rest) => {
                let (pid, scope) = rest.strip_prefix(".crash.provisional.")?.split_once('.')?;
                Some(Self::Provisional {
                    stem,
                    pid: decimal(pid)?,
                    scope: decimal(scope)?,
                })
            }
            (false, _) => None,
        }
    }

    fn temporary(hidden: &'a str) -> Option<Self> {
        let (target, suffix) = hidden.strip_suffix(".tmp")?.rsplit_once('.')?;
        let stem = Self::record(target)?.stem();
        match suffix.len() == 16 && is_lower_hex(suffix) {
            true => Some(Self::Temporary { stem }),
            false => None,
        }
    }
}

fn is_lower_hex(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `text` as a number, only when it is nothing but decimal digits.
pub(super) fn decimal(text: &str) -> Option<u64> {
    match (text.is_empty(), text.bytes().all(|b| b.is_ascii_digit())) {
        (false, true) => text.parse::<u64>().ok(),
        (true, _) | (false, false) => None,
    }
}

#[cfg(test)]
#[path = "crash_record_names_tests.rs"]
mod tests;
