//! Launch-bound parent control policy (#1935, epic #1929).
//!
//! A launcher-created harness is lifetime-scoped to the harness that launched
//! it. The launcher mints one random, generation-scoped capability per child
//! and delivers it out of band; the child accepts **exactly one** control
//! connection presenting that capability, and only the loss of that
//! connection means "my parent is gone". Every other client of the child's
//! socket (TUI, inspector, tool, probe) is ordinary and its disconnect means
//! nothing to the child's lifetime.
//!
//! This module is the pure policy: the value object for the capability, the
//! binding state machine, and the affirmative acceptance rule. Randomness,
//! files, sockets and processes stay in adapters.
use std::fmt;

use super::subagent_teardown::LaunchGeneration;

/// Length in hex characters of a capability: 256 bits of entropy.
pub const CAPABILITY_HEX_LEN: usize = 64;

/// A parent-control capability: 64 lowercase hex characters. Its `Debug`
/// and `Display` are redacted so the secret can never reach a log by
/// accident; only [`ParentControlCapability::expose`] yields the material,
/// and only adapters that deliver it out of band may call it.
#[derive(Clone, Eq)]
pub struct ParentControlCapability(String);

impl PartialEq for ParentControlCapability {
    /// Constant time over the fixed-length material, so no comparison of
    /// capabilities — not even an equality check in an adapter — can leak
    /// how many leading bytes matched.
    fn eq(&self, other: &Self) -> bool {
        self.matches(other)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityFormatError {
    WrongLength { actual: usize },
    NotLowercaseHex,
}

impl fmt::Display for CapabilityFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength { actual } => write!(
                f,
                "parent control capability must be {CAPABILITY_HEX_LEN} hex characters, got {actual}"
            ),
            Self::NotLowercaseHex => {
                f.write_str("parent control capability must be lowercase hexadecimal")
            }
        }
    }
}

impl ParentControlCapability {
    /// Affirmative parse: exactly [`CAPABILITY_HEX_LEN`] lowercase hex
    /// characters. Anything else is refused before it can be compared.
    pub fn parse(raw: &str) -> Result<Self, CapabilityFormatError> {
        if raw.len() != CAPABILITY_HEX_LEN {
            return Err(CapabilityFormatError::WrongLength { actual: raw.len() });
        }
        if !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(CapabilityFormatError::NotLowercaseHex);
        }
        Ok(Self(raw.to_owned()))
    }

    /// Build from raw random bytes rendered as lowercase hex.
    pub fn from_random_bytes(bytes: &[u8; 32]) -> Self {
        let mut out = String::with_capacity(CAPABILITY_HEX_LEN);
        for byte in bytes {
            use fmt::Write;
            write!(out, "{byte:02x}").expect("hex into String cannot fail");
        }
        Self(out)
    }

    /// The secret material. Named loudly: the only legitimate callers are
    /// the adapters that write the out-of-band sidecar and the one that
    /// presents it on the bound connection.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Constant-time equality over the fixed-length material, so a
    /// mismatching presentation cannot be narrowed byte by byte.
    fn matches(&self, other: &Self) -> bool {
        let a = self.0.as_bytes();
        let b = other.0.as_bytes();
        debug_assert_eq!(a.len(), CAPABILITY_HEX_LEN);
        debug_assert_eq!(b.len(), CAPABILITY_HEX_LEN);
        let mut diff = 0u8;
        for (x, y) in a.iter().zip(b.iter()) {
            diff |= x ^ y;
        }
        diff == 0 && a.len() == b.len()
    }
}

impl fmt::Debug for ParentControlCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ParentControlCapability(<redacted>)")
    }
}

impl fmt::Display for ParentControlCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// What a launcher hands its child out of band: the generation of this
/// launch and the capability the parent will present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentControlCredential {
    pub generation: LaunchGeneration,
    pub capability: ParentControlCapability,
}

/// Why a presentation is refused. Every variant fails closed: the presenting
/// connection is never treated as the parent and never kept as an ordinary
/// client either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindRejection {
    /// This harness was not launched with a parent binding (a top-level
    /// session): nothing can ever present as its parent.
    NotALaunchedChild,
    /// The presented generation is not this launch.
    WrongGeneration {
        expected: LaunchGeneration,
        presented: LaunchGeneration,
    },
    /// The presented capability is not the minted one.
    Mismatch,
    /// A parent is already bound (a second or replayed presentation).
    AlreadyBound,
    /// The bound parent was already lost; the binding is spent for good.
    AlreadyLost,
}

impl fmt::Display for BindRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotALaunchedChild => f.write_str("this harness has no launch-bound parent"),
            Self::WrongGeneration {
                expected,
                presented,
            } => write!(
                f,
                "parent control generation {} does not match launch generation {}",
                presented.get(),
                expected.get()
            ),
            Self::Mismatch => f.write_str("parent control capability mismatch"),
            Self::AlreadyBound => f.write_str("a parent control connection is already bound"),
            Self::AlreadyLost => f.write_str("the parent control connection was already lost"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    /// Launched with a credential, no parent bound yet.
    Unbound,
    /// Exactly one connection holds the parent role.
    Bound,
    /// The bound connection closed: the parent is gone, for good.
    Lost,
}

/// Outcome of a connection closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionLoss {
    /// The closed connection was the bound parent: common shutdown follows.
    BoundParentLost,
    /// The closed connection was not the parent: nothing follows.
    OrdinaryClient,
}

/// The one binding a launched harness holds. `None` credential means a
/// top-level harness that can never be bound.
#[derive(Debug, Clone)]
pub struct ParentControlBinding {
    expected: Option<ParentControlCredential>,
    state: BindingState,
}

impl ParentControlBinding {
    /// A harness launched with a credential to compare against.
    pub fn launched(expected: ParentControlCredential) -> Self {
        Self {
            expected: Some(expected),
            state: BindingState::Unbound,
        }
    }

    /// A top-level harness: every presentation is refused.
    pub fn unlaunched() -> Self {
        Self {
            expected: None,
            state: BindingState::Unbound,
        }
    }

    pub const fn state(&self) -> BindingState {
        self.state
    }

    pub const fn is_launched_child(&self) -> bool {
        self.expected.is_some()
    }

    /// Affirmative acceptance: the presented generation and capability must
    /// both match, and no connection may already hold (or have held) the
    /// parent role. On success the state moves to `Bound` and the caller's
    /// connection is the one whose loss means "parent gone".
    pub fn present(
        &mut self,
        generation: LaunchGeneration,
        capability: &ParentControlCapability,
    ) -> Result<(), BindRejection> {
        let Some(expected) = &self.expected else {
            return Err(BindRejection::NotALaunchedChild);
        };
        match self.state {
            BindingState::Bound => return Err(BindRejection::AlreadyBound),
            BindingState::Lost => return Err(BindRejection::AlreadyLost),
            BindingState::Unbound => {}
        }
        if expected.generation != generation {
            return Err(BindRejection::WrongGeneration {
                expected: expected.generation,
                presented: generation,
            });
        }
        if !expected.capability.matches(capability) {
            return Err(BindRejection::Mismatch);
        }
        self.state = BindingState::Bound;
        Ok(())
    }

    /// The bind deadline passed with no parent bound. Only an `Unbound`
    /// launched harness expires: the binding is spent (`Lost`) so a late
    /// presentation is refused, and `true` reports that the parent is now
    /// presumed gone. Bound, lost and top-level bindings report `false`.
    pub fn expire_unbound(&mut self) -> bool {
        if self.expected.is_some() && self.state == BindingState::Unbound {
            self.state = BindingState::Lost;
            return true;
        }
        false
    }

    /// A connection closed. Only the caller that successfully bound may pass
    /// `was_bound_connection = true`; the first such loss moves the state to
    /// `Lost` and reports it, every later or ordinary loss is nothing.
    pub fn connection_closed(&mut self, was_bound_connection: bool) -> ConnectionLoss {
        if was_bound_connection && self.state == BindingState::Bound {
            self.state = BindingState::Lost;
            return ConnectionLoss::BoundParentLost;
        }
        ConnectionLoss::OrdinaryClient
    }
}

#[cfg(test)]
#[path = "parent_control_tests.rs"]
mod tests;
