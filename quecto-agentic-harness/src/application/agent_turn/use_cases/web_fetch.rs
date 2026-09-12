//! Capability-local destination authorization for `web_fetch`.
//!
//! This policy belongs in the application layer because it is a rule of one
//! use case, not an enterprise-wide invariant. URL parsing, name resolution,
//! redirects, sockets, and HTTP remain infrastructure mechanisms; they supply
//! owned, backend-neutral facts here and proceed only after [`Allowed`](
//! WebDestinationAuthorization::Allowed).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Backend-neutral facts about one URL or one resolved connection candidate.
///
/// `effective_port` is optional deliberately: incomplete normalization must be
/// representable so that the policy can fail closed rather than invent a
/// default outside the URL mechanism. `candidate` is populated when
/// authorizing an address returned by resolution or eligible for connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebDestinationTarget {
    scheme: String,
    host: String,
    effective_port: Option<u16>,
    candidate: Option<IpAddr>,
}

impl WebDestinationTarget {
    /// Own and normalize the backend-neutral target facts.
    pub fn new(
        scheme: impl Into<String>,
        host: impl Into<String>,
        effective_port: impl Into<Option<u16>>,
        candidate: Option<IpAddr>,
    ) -> Self {
        let scheme = scheme.into().to_ascii_lowercase();
        let host = normalize_host(host.into());
        Self {
            scheme,
            host,
            effective_port: effective_port.into(),
            candidate,
        }
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn effective_port(&self) -> Option<u16> {
        self.effective_port
    }

    pub fn candidate(&self) -> Option<IpAddr> {
        self.candidate
    }

    fn has_complete_normalized_identity(&self) -> bool {
        matches!(self.effective_port, Some(1..=u16::MAX))
            && self.scheme == self.scheme.to_ascii_lowercase()
            && self.host == normalize_host(self.host.clone())
            && !self.host.is_empty()
    }
}

/// The only decisions on which `web_fetch` execution may proceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "network execution requires an explicit Allowed decision"]
pub enum WebDestinationAuthorization {
    Allowed,
    Denied,
}

/// Sole application authority for `web_fetch` schemes and destinations.
#[derive(Clone, Debug, Default)]
pub struct WebDestinationPolicy {
    #[cfg(any(test, feature = "test-support"))]
    test_destination: Option<TestDestination>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct TestDestination {
    host: String,
    port: u16,
    candidate: IpAddr,
}

impl WebDestinationPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Permit exactly one normalized host/port destination in test builds.
    ///
    /// This is intentionally not a wildcard. It exists only so deterministic
    /// tests can use a local server while all other restricted destinations
    /// remain denied. Scheme authorization is never bypassed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_test_destination(host: impl Into<String>, port: u16, candidate: IpAddr) -> Self {
        Self {
            test_destination: Some(TestDestination {
                host: normalize_host(host.into()),
                port,
                candidate,
            }),
        }
    }

    /// Authorize a normalized URL target or resolved connection candidate.
    ///
    /// Authorization is conjunctive and affirmative: every required fact must
    /// be classified, and a supplied address candidate must itself be public
    /// (or belong to the exact test-only destination).
    pub fn authorize(&self, target: &WebDestinationTarget) -> WebDestinationAuthorization {
        let test_identity_allowed = self.test_identity_allowed(target);
        let test_candidate_allowed = self.test_candidate_allowed(target);
        let identity_allowed = target.has_complete_normalized_identity()
            && is_allowed_scheme(&target.scheme)
            && (is_public_host(&target.host) || test_identity_allowed);
        let candidate_allowed = target
            .candidate
            .is_none_or(|candidate| is_public_ip(candidate) || test_candidate_allowed);

        if identity_allowed && candidate_allowed {
            WebDestinationAuthorization::Allowed
        } else {
            WebDestinationAuthorization::Denied
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn test_identity_allowed(&self, target: &WebDestinationTarget) -> bool {
        self.test_destination.as_ref().is_some_and(|allowed| {
            allowed.port > 0
                && target.host == allowed.host
                && target.effective_port == Some(allowed.port)
                && is_well_formed_host(&allowed.host)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    fn test_candidate_allowed(&self, target: &WebDestinationTarget) -> bool {
        self.test_destination.as_ref().is_some_and(|allowed| {
            self.test_identity_allowed(target) && target.candidate == Some(allowed.candidate)
        })
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn test_identity_allowed(&self, _target: &WebDestinationTarget) -> bool {
        false
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn test_candidate_allowed(&self, _target: &WebDestinationTarget) -> bool {
        false
    }
}

fn normalize_host(host: String) -> String {
    let lowercase = host.to_ascii_lowercase();
    lowercase
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .filter(|inner| inner.parse::<Ipv6Addr>().is_ok())
        .unwrap_or(&lowercase)
        .to_owned()
}

fn is_allowed_scheme(scheme: &str) -> bool {
    matches!(scheme, "http" | "https")
}

fn is_public_host(host: &str) -> bool {
    host.parse::<IpAddr>()
        .map(is_public_ip)
        .unwrap_or_else(|_| is_public_dns_name(host))
}

#[cfg(any(test, feature = "test-support"))]
fn is_well_formed_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok() || is_well_formed_dns_name(host)
}

fn is_public_dns_name(host: &str) -> bool {
    is_well_formed_dns_name(host)
        && !matches!(
            host,
            "localhost" | "localhost." | "metadata.google.internal" | "metadata.google.internal."
        )
}

fn is_well_formed_dns_name(host: &str) -> bool {
    let without_root = host.strip_suffix('.').unwrap_or(host);
    let labels: Vec<_> = without_root.split('.').collect();
    let has_public_shape = without_root.len() <= 253
        && labels.len() >= 2
        && !without_root
            .bytes()
            .all(|byte| byte == b'.' || byte.is_ascii_digit());

    has_public_shape
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    // Affirmative IANA globally-reachable IPv4 intervals. Values outside
    // these deliberately bounded intervals are denied by default, so a new
    // or unclassified special-purpose allocation cannot fail open merely
    // because it was absent from a denylist.
    const GLOBALLY_REACHABLE: &[(u32, u32)] = &[
        (0x0100_0000, 0x09ff_ffff), // 1/8 through 9/8
        (0x0b00_0000, 0x643f_ffff), // 11/8 through before 100.64/10
        (0x6480_0000, 0x7eff_ffff), // after 100.64/10 through 126/8
        (0x8000_0000, 0xa9fd_ffff), // 128/8 through before 169.254/16
        (0xa9ff_0000, 0xac0f_ffff), // after 169.254/16 through before 172.16/12
        (0xac20_0000, 0xbfff_ffff), // after 172.16/12 through 191/8
        (0xc000_0009, 0xc000_000a), // explicitly global PCP/TURN anycast
        (0xc000_0100, 0xc000_01ff), // 192.0.1/24
        (0xc000_0300, 0xc058_62ff), // after TEST-NET-1 through before 6to4 relay
        (0xc058_6400, 0xc0a7_ffff), // after 192.88.99/24 through before RFC1918
        (0xc0a9_0000, 0xc611_ffff), // after 192.168/16 through before benchmark
        (0xc614_0000, 0xc633_63ff), // after benchmark through before TEST-NET-2
        (0xc633_6500, 0xcb00_70ff), // after TEST-NET-2 through before TEST-NET-3
        (0xcb00_7200, 0xdfff_ffff), // after TEST-NET-3 through unicast /4
    ];

    let value = u32::from(ip);
    GLOBALLY_REACHABLE
        .iter()
        .any(|&(first, last)| (first..=last).contains(&value))
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    // Affirmative IANA-allocated, globally-reachable unicast intervals. The
    // global-unicast envelope (2000::/3) is not itself an allocation: every
    // unlisted hole is reserved and therefore denied by default. Special-use
    // prefixes are admitted only where IANA explicitly marks them globally
    // reachable. 6to4 (2002::/16) remains absent because its reachability is
    // conditional on an embedded IPv4 destination.
    const GLOBALLY_REACHABLE: &[(u128, u128)] = &[
        // Globally-reachable special-purpose assignments in 2001::/23.
        (
            0x20010001000000000000000000000001,
            0x20010001000000000000000000000003,
        ),
        (
            0x20010003000000000000000000000000,
            0x20010003ffffffffffffffffffffffff,
        ),
        (
            0x20010004011200000000000000000000,
            0x200100040112ffffffffffffffffffff,
        ),
        (
            0x20010020000000000000000000000000,
            0x2001002fffffffffffffffffffffffff,
        ),
        (
            0x20010030000000000000000000000000,
            0x2001003fffffffffffffffffffffffff,
        ),
        // IANA regional allocations. Gaps between these entries remain denied.
        (
            0x20010200000000000000000000000000,
            0x200103ffffffffffffffffffffffffff,
        ),
        (
            0x20010400000000000000000000000000,
            0x200105ffffffffffffffffffffffffff,
        ),
        (
            0x20010600000000000000000000000000,
            0x200107ffffffffffffffffffffffffff,
        ),
        (
            0x20010800000000000000000000000000,
            0x20010bffffffffffffffffffffffffff,
        ),
        (
            0x20010c00000000000000000000000000,
            0x20010db7ffffffffffffffffffffffff,
        ),
        (
            0x20010db9000000000000000000000000,
            0x20010dffffffffffffffffffffffffff,
        ),
        (
            0x20010e00000000000000000000000000,
            0x20010fffffffffffffffffffffffffff,
        ),
        (
            0x20011200000000000000000000000000,
            0x200113ffffffffffffffffffffffffff,
        ),
        (
            0x20011400000000000000000000000000,
            0x200117ffffffffffffffffffffffffff,
        ),
        (
            0x20011800000000000000000000000000,
            0x200119ffffffffffffffffffffffffff,
        ),
        (
            0x20011a00000000000000000000000000,
            0x20011bffffffffffffffffffffffffff,
        ),
        (
            0x20011c00000000000000000000000000,
            0x20011fffffffffffffffffffffffffff,
        ),
        (
            0x20012000000000000000000000000000,
            0x20013fffffffffffffffffffffffffff,
        ),
        (
            0x20014000000000000000000000000000,
            0x200141ffffffffffffffffffffffffff,
        ),
        (
            0x20014200000000000000000000000000,
            0x200143ffffffffffffffffffffffffff,
        ),
        (
            0x20014400000000000000000000000000,
            0x200145ffffffffffffffffffffffffff,
        ),
        (
            0x20014600000000000000000000000000,
            0x200147ffffffffffffffffffffffffff,
        ),
        (
            0x20014800000000000000000000000000,
            0x200149ffffffffffffffffffffffffff,
        ),
        (
            0x20014a00000000000000000000000000,
            0x20014bffffffffffffffffffffffffff,
        ),
        (
            0x20014c00000000000000000000000000,
            0x20014dffffffffffffffffffffffffff,
        ),
        (
            0x20015000000000000000000000000000,
            0x20015fffffffffffffffffffffffffff,
        ),
        (
            0x20018000000000000000000000000000,
            0x20019fffffffffffffffffffffffffff,
        ),
        (
            0x2001a000000000000000000000000000,
            0x2001afffffffffffffffffffffffffff,
        ),
        (
            0x2001b000000000000000000000000000,
            0x2001bfffffffffffffffffffffffffff,
        ),
        (
            0x20030000000000000000000000000000,
            0x20033fffffffffffffffffffffffffff,
        ),
        (
            0x24000000000000000000000000000000,
            0x241fffffffffffffffffffffffffffff,
        ),
        (
            0x26000000000000000000000000000000,
            0x260fffffffffffffffffffffffffffff,
        ),
        (
            0x26100000000000000000000000000000,
            0x2611ffffffffffffffffffffffffffff,
        ),
        (
            0x26200000000000000000000000000000,
            0x2621ffffffffffffffffffffffffffff,
        ),
        (
            0x26300000000000000000000000000000,
            0x263fffffffffffffffffffffffffffff,
        ),
        (
            0x28000000000000000000000000000000,
            0x280fffffffffffffffffffffffffffff,
        ),
        (
            0x2a000000000000000000000000000000,
            0x2a1fffffffffffffffffffffffffffff,
        ),
        (
            0x2c000000000000000000000000000000,
            0x2c0fffffffffffffffffffffffffffff,
        ),
    ];

    let value = u128::from(ip);
    GLOBALLY_REACHABLE
        .iter()
        .any(|&(first, last)| (first..=last).contains(&value))
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
