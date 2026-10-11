//! Who holds a task: a git committer identity and the claim's window.
use super::super::value_objects::schema_error::{SchemaError, line};
use super::super::value_objects::timestamp::Timestamp;
use regex::Regex;
use std::sync::LazyLock;

/// A claim lasts at most two hours; its holder renews it.
pub const CLAIM_TTL_SECONDS: u64 = 2 * 60 * 60;
pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_EMAIL_BYTES: usize = 254;

/// Who holds the task, from when, until when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub holder: Identity,
    pub since: Timestamp,
    pub expires: Timestamp,
}

/// A git committer identity, as the store is given it. It is self-asserted
/// (anyone can commit as any name and email), so it attributes board
/// writes; it does not authenticate them. Two identities are the same
/// person, and equal (`==`), when their emails match ignoring ASCII case:
/// the name is only how they are shown.
#[derive(Debug, Clone)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

impl PartialEq for Identity {
    /// Agrees with [`Identity::is`]: the same committer, whatever the name.
    fn eq(&self, other: &Self) -> bool {
        self.is(other)
    }
}

/// Equal emails ignoring ASCII case is an equivalence.
impl Eq for Identity {}

/// A name is one line of text (checked by `line`) that begins and ends
/// with a visible character and holds no angle bracket, git's own
/// delimiter around the email: `dependabot[bot]`, `Ada Jr.` and
/// `Shane (work)` are names; ` Ada` and `Ada <a@b>` are not.
static NAME: LazyLock<Regex> = LazyLock::new(|| {
    let edge = r"[[\p{L}\p{M}\p{N}\p{P}\p{S}\x{E007F}]--[<>]]";
    let inner = r"[[\p{L}\p{M}\p{N}\p{P}\p{S}\p{Zs}\x{200C}\x{200D}\x{E0020}-\x{E007F}]--[<>]]";
    Regex::new(&format!("^{edge}(?:{inner}*{edge})?$")).expect("name allowlist")
});

impl Claim {
    /// The holder is a valid identity; the claim ends after it starts and
    /// lasts no longer than [`CLAIM_TTL_SECONDS`].
    pub fn validate(&self, field: &str) -> Result<(), SchemaError> {
        self.holder.validate(&format!("{field}/holder"))?;
        let cap = self
            .since
            .plus_seconds(CLAIM_TTL_SECONDS)
            .map_err(|error| error.at(field))?;
        if self.since < self.expires && self.expires <= cap {
            Ok(())
        } else {
            Err(SchemaError::new(
                field,
                "must expire after it starts and within two hours",
            ))
        }
    }
}

impl Identity {
    pub fn validate(&self, field: &str) -> Result<(), SchemaError> {
        let name = format!("{field}/name");
        line(&name, &self.name, MAX_NAME_CHARS)?;
        if !NAME.is_match(&self.name) {
            return Err(SchemaError::new(
                name,
                "must start and end with a visible character, without < or >",
            ));
        }
        let shaped = self.email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        });
        let allowed = |b: u8| b.is_ascii_graphic() && !matches!(b, b'<' | b'>');
        if shaped && self.email.len() <= MAX_EMAIL_BYTES && self.email.bytes().all(allowed) {
            Ok(())
        } else {
            Err(SchemaError::new(
                format!("{field}/email"),
                "must be local@domain in printable ASCII",
            ))
        }
    }

    /// The same committer: emails equal ignoring ASCII case.
    pub fn is(&self, other: &Identity) -> bool {
        self.email.eq_ignore_ascii_case(&other.email)
    }
}
