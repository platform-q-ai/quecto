//! git's command-scope configuration a caller passes in its environment
//! (`GIT_CONFIG_COUNT` with `GIT_CONFIG_KEY_<n>` and
//! `GIT_CONFIG_VALUE_<n>`), which outranks the repository's own: how a
//! caller such as the pocket relay, or quecto's own container scripts,
//! names the committer the board writes as, whatever `user.name` and
//! `user.email` the clone configures. git takes each key on its own: a
//! set naming `user.name` alone leaves `user.email` to the repository,
//! which [`CommandScopeWarning::HalfIdentity`] tells.
//!
//! Only an identity passes: the settings of the set whose key is one of
//! [`COMMAND_CONFIG_KEYS`], matched as git matches a two-part key (in any
//! ASCII case) and passed on in lower case, renumbered from 0. Every other
//! setting is skipped, quietly. Command-scope configuration could set
//! `core.sshCommand`, `core.hooksPath`, `credential.helper` and the like,
//! which run programs; a caller that controls the environment already has
//! that power through `GIT_CONFIG_GLOBAL`, so passing them would add none,
//! but the board's use for the scope is naming its committer, and nothing
//! else of it reaches git. A setting is skipped silently because a key can
//! carry a credential (a `url.<base>.insteadOf` with a token in it): no key
//! or value is ever quoted. A key git itself would refuse (an empty one,
//! one with a trailing space) is skipped the same way, never passed and
//! never the reason a set is dropped, so git never gets to quote it in an
//! error of its own.
//!
//! The set is read as git reads it, its count a little more strictly:
//! - the count must be empty (no set) or plain ASCII decimal (git's
//!   `strtoul` also takes leading spaces and a sign, which drop the set
//!   here);
//! - a count above [`MAX_COUNT`] (`INT_MAX`), which git refuses ("too many
//!   entries"), drops the set;
//! - every setting below the count must have its key and its value, as git
//!   dies otherwise.
//!
//! A set read otherwise is dropped whole, and why ([`CommandScopeWarning`],
//! which names only a variable) is the front end's to warn of. The board
//! carries on as it would with no set; git itself would refuse to run.
//!
//! However many settings a set carries, its identity is read (#2703: the
//! pocket relay's scope carries 41, its bot committer among them), in index
//! order, so the last of a key wins as in git. The work is bounded by the
//! environment's own size, whatever the count: each setting is looked up
//! once, and a count past the settings present stops at the first missing
//! one, which is never past the number of keys set.
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt;

/// The largest count git takes (`INT_MAX`): a larger one it refuses.
pub const MAX_COUNT: usize = 2_147_483_647;

/// The keys whose settings pass, in the lower case they pass in: the
/// committer the board writes as. The board names a commit's author and
/// committer both from these (as `git config` resolves them) and passes
/// them to git explicitly, so `author.*` and `committer.*` would change
/// nothing, and are skipped.
pub const COMMAND_CONFIG_KEYS: &[&str] = &["user.name", "user.email"];

const COUNT: &str = "GIT_CONFIG_COUNT";

/// What a command-scope set warns of. It names variables and the
/// identity's keys, in words the board chose: never a key or a value the
/// caller set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandScopeWarning {
    /// Dropped: `GIT_CONFIG_COUNT` is neither empty nor plain decimal 0 to
    /// [`MAX_COUNT`].
    Count,
    /// Dropped: a setting's key or value variable is not set.
    Missing { variable: String },
    /// Passed: the set names one of the identity's keys and not the
    /// other, which comes from the repository's own configuration.
    HalfIdentity {
        named: &'static str,
        unnamed: &'static str,
    },
}

impl fmt::Display for CommandScopeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scope = "git's command-scope configuration (GIT_CONFIG_COUNT)";
        match self {
            Self::Count => write!(
                f,
                "ignoring {scope}: {COUNT} is not plain decimal 0 to {MAX_COUNT}"
            ),
            Self::Missing { variable } => write!(f, "ignoring {scope}: {variable} is not set"),
            Self::HalfIdentity { named, unnamed } => write!(
                f,
                "{scope} sets {named} but not {unnamed}, which comes from the repository's own"
            ),
        }
    }
}

/// What of a command-scope set passes to git, and what it warns of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommandScope {
    pub vars: Vec<(OsString, OsString)>,
    pub warning: Option<CommandScopeWarning>,
}

/// The command-scope variables of `vars` to pass to git: the set's
/// identity settings in index order, renumbered from 0 under a count of
/// their own; none when there is no set, none of its settings is an
/// identity's, or it is dropped (with why).
pub(super) fn command_config(vars: &[(OsString, OsString)]) -> CommandScope {
    let dropped = |why| CommandScope {
        vars: Vec::new(),
        warning: Some(why),
    };
    // The scope's variables by name, each looked up once; the first of a
    // name wins, as a linear search would find it.
    let mut scope: HashMap<&OsStr, &OsStr> = HashMap::new();
    for (key, value) in vars {
        if key.as_encoded_bytes().starts_with(b"GIT_CONFIG_") {
            scope.entry(key.as_os_str()).or_insert(value.as_os_str());
        }
    }
    let get = |name: &str| scope.get(OsStr::new(name)).copied();
    let count = match get(COUNT).map(count_of) {
        Some(Some(count)) => count,
        Some(None) => return dropped(CommandScopeWarning::Count),
        None => 0,
    };
    // The keys present bound the walk: setting `n` can be whole only when
    // `n` keys before it are set, so the first missing one comes by then.
    let keys = scope
        .keys()
        .filter(|name| name.as_encoded_bytes().starts_with(b"GIT_CONFIG_KEY_"))
        .count();
    let mut identity: Vec<(&'static str, &OsStr)> = Vec::new();
    for n in 0..count {
        assert!(
            n <= keys,
            "a walk past the keys present has met a missing one"
        );
        let setting = Setting::numbered(n, get);
        match (setting.key.map(identity_key), setting.value) {
            (Some(Some(key)), Some(value)) => identity.push((key, value)),
            (Some(None), Some(_)) => {}
            (None, _) => {
                return dropped(CommandScopeWarning::Missing {
                    variable: setting.key_name,
                });
            }
            (Some(_), None) => {
                return dropped(CommandScopeWarning::Missing {
                    variable: setting.value_name,
                });
            }
        }
    }
    let names = |key: &str| identity.iter().any(|(named, _)| *named == key);
    let [name, email] = [COMMAND_CONFIG_KEYS[0], COMMAND_CONFIG_KEYS[1]];
    let warning = match (names(name), names(email)) {
        (true, false) => Some(CommandScopeWarning::HalfIdentity {
            named: name,
            unnamed: email,
        }),
        (false, true) => Some(CommandScopeWarning::HalfIdentity {
            named: email,
            unnamed: name,
        }),
        (true, true) | (false, false) => None,
    };
    let renumbered = identity.iter().enumerate().flat_map(|(n, (key, value))| {
        [
            (format!("GIT_CONFIG_KEY_{n}").into(), OsString::from(key)),
            (format!("GIT_CONFIG_VALUE_{n}").into(), (*value).to_owned()),
        ]
    });
    let passed: Vec<(OsString, OsString)> = match identity.len() {
        0 => Vec::new(),
        kept => std::iter::once((OsString::from(COUNT), OsString::from(kept.to_string())))
            .chain(renumbered)
            .collect(),
    };
    debug_assert!(identity.len() <= count && count <= keys && count <= MAX_COUNT);
    debug_assert!(passed.len() == 2 * identity.len() + usize::from(!identity.is_empty()));
    CommandScope {
        vars: passed,
        warning,
    }
}

/// Setting `n` of a set: its variables' names, and their values when set.
struct Setting<'a> {
    key_name: String,
    key: Option<&'a OsStr>,
    value_name: String,
    value: Option<&'a OsStr>,
}

impl<'a> Setting<'a> {
    fn numbered(n: usize, get: impl Fn(&str) -> Option<&'a OsStr>) -> Self {
        let (key_name, value_name) = (
            format!("GIT_CONFIG_KEY_{n}"),
            format!("GIT_CONFIG_VALUE_{n}"),
        );
        Self {
            key: get(&key_name),
            value: get(&value_name),
            key_name,
            value_name,
        }
    }
}

/// A count: empty (no set, as git reads it), or plain ASCII decimal
/// digits at most [`MAX_COUNT`].
fn count_of(text: &OsStr) -> Option<usize> {
    let text = text.to_str()?;
    let count = match text {
        "" => 0,
        digits => digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse::<usize>().ok())
            .flatten()?,
    };
    (count <= MAX_COUNT).then_some(count)
}

/// The one of [`COMMAND_CONFIG_KEYS`] that `key` names, matched in any
/// ASCII case as git matches a two-part key; `None` for any other key,
/// one that is not UTF-8 included.
fn identity_key(key: &OsStr) -> Option<&'static str> {
    let key = key.to_str()?;
    COMMAND_CONFIG_KEYS
        .iter()
        .copied()
        .find(|allowed| allowed.eq_ignore_ascii_case(key))
}
