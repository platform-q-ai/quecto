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
    pub fn with_test_destination(host: impl Into<String>, port: u16) -> Self {
        Self {
            test_destination: Some(TestDestination {
                host: normalize_host(host.into()),
                port,
            }),
        }
    }

    /// Authorize a normalized URL target or resolved connection candidate.
    ///
    /// Authorization is conjunctive and affirmative: every required fact must
    /// be classified, and a supplied address candidate must itself be public
    /// (or belong to the exact test-only destination).
    pub fn authorize(&self, target: &WebDestinationTarget) -> WebDestinationAuthorization {
        let test_destination_allowed = self.test_destination_allowed(target);
        let identity_allowed = target.has_complete_normalized_identity()
            && is_allowed_scheme(&target.scheme)
            && (is_public_host(&target.host) || test_destination_allowed);
        let candidate_allowed = target
            .candidate
            .is_none_or(|candidate| is_public_ip(candidate) || test_destination_allowed);

        if identity_allowed && candidate_allowed {
            WebDestinationAuthorization::Allowed
        } else {
            WebDestinationAuthorization::Denied
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn test_destination_allowed(&self, target: &WebDestinationTarget) -> bool {
        self.test_destination.as_ref().is_some_and(|allowed| {
            allowed.port > 0
                && target.host == allowed.host
                && target.effective_port == Some(allowed.port)
                && is_well_formed_host(&allowed.host)
        })
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn test_destination_allowed(&self, _target: &WebDestinationTarget) -> bool {
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
    // Public unicast is the complement of the IANA special-purpose blocks.
    // Keep this classification here, alongside the scheme rule, so adapters
    // cannot quietly acquire a second destination authority.
    !(ip.is_unspecified()
        || ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || ipv4_in_cidr(ip, Ipv4Addr::new(0, 0, 0, 0), 8)
        || ipv4_in_cidr(ip, Ipv4Addr::new(100, 64, 0, 0), 10)
        || ipv4_in_cidr(ip, Ipv4Addr::new(192, 0, 0, 0), 24)
        || ipv4_in_cidr(ip, Ipv4Addr::new(192, 88, 99, 0), 24)
        || ipv4_in_cidr(ip, Ipv4Addr::new(198, 18, 0, 0), 15)
        || ipv4_in_cidr(ip, Ipv4Addr::new(240, 0, 0, 0), 4))
}

fn ipv4_in_cidr(ip: Ipv4Addr, network: Ipv4Addr, prefix: u32) -> bool {
    assert!(prefix <= 32, "IPv4 prefix invariant");
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    u32::from(ip) & mask == u32::from(network) & mask
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    // Globally routable unicast currently occupies 2000::/3. Explicitly
    // remove documentation and special assignment blocks from that positive
    // classification; all other IPv6 classes fail closed.
    ipv6_in_cidr(ip, Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3)
        && !ipv6_in_cidr(ip, Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23)
        && !ipv6_in_cidr(ip, Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0), 32)
        && !ipv6_in_cidr(ip, Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20)
}

fn ipv6_in_cidr(ip: Ipv6Addr, network: Ipv6Addr, prefix: u32) -> bool {
    assert!(prefix <= 128, "IPv6 prefix invariant");
    let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
    u128::from(ip) & mask == u128::from(network) & mask
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
