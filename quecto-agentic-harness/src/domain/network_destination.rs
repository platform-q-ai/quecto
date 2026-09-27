//! Which network destinations an outbound fetch may reach (#1942).
//!
//! One affirmative rule: an address is a permitted destination only when it
//! is globally routable unicast. Everything else — private, loopback,
//! link-local (the cloud metadata service), shared, documentation,
//! benchmarking, multicast, reserved and unallocated space — is refused by
//! default. IPv6 forms that stand for an IPv4 address (IPv4-mapped, NAT64,
//! 6to4, Teredo) are judged by the IPv4 address they reach, so a private
//! IPv4 address cannot be smuggled in an IPv6 spelling. Pure; no I/O.
//!
//! The address allowlist is the security boundary. [`is_fetchable_name`]
//! is not: it is a courtesy check over a short list of local names, refused
//! early with a plain reason, and a list of names can never be complete.
//! Every address any name resolves to is still judged by the allowlist.
//!
//! The allowlist is the unicast range (IPv4) and the space IANA has
//! allocated to the registries (IPv6). The `*_NOT_GLOBAL` tables are not a
//! denylist of the whole address space: they are the special-purpose
//! carve-outs IANA registers inside that allowed space (private, loopback,
//! link-local, documentation, benchmarking, ...), which the allowlist's own
//! ranges contain and so must take back out. An address outside the allowed
//! ranges is refused whether or not any table names it.
//!
//! Residual risk: only the well-known NAT64 prefix `64:ff9b::/96` is
//! recognised. A network-specific NAT64 prefix (RFC 6052) inside allocated
//! registry space looks like any public IPv6 address, so on an IPv6-only
//! host behind such a translator an embedded private IPv4 address could be
//! reached through it.
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// A refused destination: the address as written and, when it is an IPv6
/// form of an IPv4 address, the IPv4 address that decided the refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NonPublicAddress {
    address: IpAddr,
    embedded: Option<Ipv4Addr>,
}

impl NonPublicAddress {
    pub fn address(&self) -> IpAddr {
        self.address
    }

    pub fn embedded(&self) -> Option<Ipv4Addr> {
        self.embedded
    }
}

impl std::fmt::Display for NonPublicAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.embedded {
            Some(v4) => write!(f, "{v4} (via {}) is not a public address", self.address),
            None => write!(f, "{} is not a public address", self.address),
        }
    }
}

impl std::error::Error for NonPublicAddress {}

/// Admits `address` only when it is globally routable unicast, the only
/// kind of destination an outbound fetch may reach; otherwise says why not.
pub fn authorize_destination(address: IpAddr) -> Result<(), NonPublicAddress> {
    let refused = |embedded| NonPublicAddress { address, embedded };
    match address {
        IpAddr::V4(v4) if is_public_v4(v4) => Ok(()),
        IpAddr::V4(_) => Err(refused(None)),
        IpAddr::V6(v6) => match embedded_ipv4(v6) {
            Embedded::Judged(reached) => match reached.iter().find(|v4| !is_public_v4(**v4)) {
                None => Ok(()),
                Some(private) => Err(refused(Some(*private))),
            },
            Embedded::Absent if is_public_v6(v6) => Ok(()),
            Embedded::Absent => Err(refused(None)),
        },
    }
}

/// Whether a URL may give `name` as its host: any name outside the
/// courtesy list of local names ([`is_local_name`]).
pub fn is_fetchable_name(name: &str) -> bool {
    !is_local_name(name)
}

/// A courtesy, not the security boundary: names that mean this machine or a
/// cloud metadata service, refused early with a plain reason (#1942).
/// Without a proxy it adds nothing (every address such a name resolves to
/// is refused by [`authorize_destination`] anyway); through an HTTP(S)
/// proxy, which resolves names itself, it is the only name check there is,
/// and a list of names can never be complete. Case and any trailing dots
/// are ignored.
fn is_local_name(name: &str) -> bool {
    const LOCAL_NAMES: &[&str] = &[
        "localhost",
        "localhost.localdomain",
        "metadata",
        "metadata.goog",
        "metadata.google.internal",
    ];
    let bare = name.trim_end_matches('.').to_ascii_lowercase();
    LOCAL_NAMES.contains(&bare.as_str()) || bare.ends_with(".localhost")
}

/// An IPv4 block, `base/prefix`.
struct V4Block(Ipv4Addr, u8);

impl V4Block {
    fn contains(&self, address: Ipv4Addr) -> bool {
        let mask = u32::MAX.checked_shl(32 - u32::from(self.1)).unwrap_or(0);
        u32::from(address) & mask == u32::from(self.0) & mask
    }
}

/// An IPv6 block, `base/prefix`.
struct V6Block(Ipv6Addr, u8);

impl V6Block {
    fn contains(&self, address: Ipv6Addr) -> bool {
        let mask = u128::MAX.checked_shl(128 - u32::from(self.1)).unwrap_or(0);
        u128::from(address) & mask == u128::from(self.0) & mask
    }
}

const fn v6(segments: [u16; 8]) -> Ipv6Addr {
    let [a, b, c, d, e, f, g, h] = segments;
    Ipv6Addr::new(a, b, c, d, e, f, g, h)
}

/// Special-purpose blocks inside the unicast range that are not globally
/// reachable (IANA IPv4 Special-Purpose Address Registry).
const V4_NOT_GLOBAL: &[V4Block] = &[
    V4Block(Ipv4Addr::new(10, 0, 0, 0), 8),
    V4Block(Ipv4Addr::new(100, 64, 0, 0), 10),
    V4Block(Ipv4Addr::new(127, 0, 0, 0), 8),
    V4Block(Ipv4Addr::new(169, 254, 0, 0), 16),
    V4Block(Ipv4Addr::new(172, 16, 0, 0), 12),
    V4Block(Ipv4Addr::new(192, 0, 0, 0), 24),
    V4Block(Ipv4Addr::new(192, 0, 2, 0), 24),
    V4Block(Ipv4Addr::new(192, 88, 99, 0), 24),
    V4Block(Ipv4Addr::new(192, 168, 0, 0), 16),
    V4Block(Ipv4Addr::new(198, 18, 0, 0), 15),
    V4Block(Ipv4Addr::new(198, 51, 100, 0), 24),
    V4Block(Ipv4Addr::new(203, 0, 113, 0), 24),
];

/// Unicast IPv4 (1.0.0.0 to 223.255.255.255: not `0/8`, multicast,
/// reserved or broadcast) outside every special-purpose block.
fn is_public_v4(address: Ipv4Addr) -> bool {
    let unicast = (1..=223).contains(&address.octets()[0]);
    unicast && V4_NOT_GLOBAL.iter().all(|block| !block.contains(address))
}

/// Global unicast space IANA has allocated to the regional registries
/// (IANA IPv6 Global Unicast Address Assignments). Holes and future space
/// are refused until they are allocated. `2002::/16` (6to4) is judged by its
/// embedded IPv4 address instead.
const V6_ALLOCATED: &[V6Block] = &[
    V6Block(v6([0x2001, 0, 0, 0, 0, 0, 0, 0]), 16),
    V6Block(v6([0x2003, 0, 0, 0, 0, 0, 0, 0]), 18),
    V6Block(v6([0x2400, 0, 0, 0, 0, 0, 0, 0]), 12),
    V6Block(v6([0x2600, 0, 0, 0, 0, 0, 0, 0]), 12),
    V6Block(v6([0x2610, 0, 0, 0, 0, 0, 0, 0]), 23),
    V6Block(v6([0x2620, 0, 0, 0, 0, 0, 0, 0]), 23),
    V6Block(v6([0x2630, 0, 0, 0, 0, 0, 0, 0]), 12),
    V6Block(v6([0x2800, 0, 0, 0, 0, 0, 0, 0]), 12),
    V6Block(v6([0x2a00, 0, 0, 0, 0, 0, 0, 0]), 12),
    V6Block(v6([0x2a10, 0, 0, 0, 0, 0, 0, 0]), 12),
    V6Block(v6([0x2c00, 0, 0, 0, 0, 0, 0, 0]), 12),
];

/// Special-purpose blocks inside the allocated space that are not globally
/// reachable: the IETF protocol block (benchmarking, ORCHID, ...; Teredo is
/// judged by its embedded addresses before this) and documentation.
const V6_NOT_GLOBAL: &[V6Block] = &[
    V6Block(v6([0x2001, 0, 0, 0, 0, 0, 0, 0]), 23),
    V6Block(v6([0x2001, 0x0db8, 0, 0, 0, 0, 0, 0]), 32),
];

fn is_public_v6(address: Ipv6Addr) -> bool {
    V6_ALLOCATED.iter().any(|block| block.contains(address))
        && V6_NOT_GLOBAL.iter().all(|block| !block.contains(address))
}

/// What an IPv6 address says about IPv4.
enum Embedded {
    /// An IPv6 form that reaches these IPv4 addresses: every one must be
    /// public for the address to be.
    Judged(Vec<Ipv4Addr>),
    /// A native IPv6 address, judged by the IPv6 rules.
    Absent,
}

const MAPPED: V6Block = V6Block(v6([0, 0, 0, 0, 0, 0xffff, 0, 0]), 96);
const NAT64: V6Block = V6Block(v6([0x64, 0xff9b, 0, 0, 0, 0, 0, 0]), 96);
const SIX_TO_FOUR: V6Block = V6Block(v6([0x2002, 0, 0, 0, 0, 0, 0, 0]), 16);
const TEREDO: V6Block = V6Block(v6([0x2001, 0, 0, 0, 0, 0, 0, 0]), 32);

fn embedded_ipv4(address: Ipv6Addr) -> Embedded {
    let bits = u128::from(address);
    // Every shift leaves at most 32 significant bits, so each cast is exact.
    let low = Ipv4Addr::from(bits as u32);
    if MAPPED.contains(address) || NAT64.contains(address) {
        Embedded::Judged(vec![low])
    } else if SIX_TO_FOUR.contains(address) {
        Embedded::Judged(vec![Ipv4Addr::from((bits >> 80) as u32)])
    } else if TEREDO.contains(address) {
        // Server in bits 32..64, client obfuscated (inverted) in the last 32.
        let server = Ipv4Addr::from((bits >> 64) as u32);
        Embedded::Judged(vec![server, Ipv4Addr::from(!(bits as u32))])
    } else {
        // IPv4-compatible (`::a.b.c.d`, deprecated) lies outside the
        // allocated space, so the IPv6 rules refuse it like every other form
        // there.
        Embedded::Absent
    }
}

#[cfg(test)]
#[path = "network_destination_tests.rs"]
mod tests;
