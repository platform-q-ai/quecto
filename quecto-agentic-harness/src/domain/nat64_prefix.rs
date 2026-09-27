//! Network-specific NAT64 prefixes (RFC 6052) and their discovery answers
//! (RFC 7050), for judging an IPv6 address by the IPv4 address a
//! translator reaches through it (#1942). Pure; no I/O: the adapter looks
//! up `ipv4only.arpa` and hands the answers here.
use core::net::{Ipv4Addr, Ipv6Addr};

/// The name whose AAAA answers reveal a DNS64 translator's prefixes.
pub const DISCOVERY_NAME: &str = "ipv4only.arpa";

/// The well-known IPv4 addresses of [`DISCOVERY_NAME`] (RFC 7050 §2.2).
const WELL_KNOWN_IPV4: [Ipv4Addr; 2] =
    [Ipv4Addr::new(192, 0, 0, 170), Ipv4Addr::new(192, 0, 0, 171)];

/// The prefix lengths RFC 6052 §2.2 defines.
const LENGTHS: [u8; 6] = [32, 40, 48, 56, 64, 96];

/// Octet 8 (bits 64-71) is the reserved "u" octet: RFC 6052 never places
/// IPv4 bits there.
const U_OCTET: usize = 8;

/// A NAT64 prefix of one of the RFC 6052 lengths, its host bits cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Nat64Prefix {
    prefix: Ipv6Addr,
    length: u8,
}

impl Nat64Prefix {
    /// `prefix/length`, when `length` is an RFC 6052 length.
    pub fn new(prefix: Ipv6Addr, length: u8) -> Option<Self> {
        LENGTHS.contains(&length).then(|| Self {
            prefix: masked(prefix, length),
            length,
        })
    }

    pub fn prefix(&self) -> Ipv6Addr {
        self.prefix
    }

    pub fn length(&self) -> u8 {
        self.length
    }

    /// Whether `address` lies under this prefix.
    pub fn contains(&self, address: Ipv6Addr) -> bool {
        masked(address, self.length) == self.prefix
    }

    /// The octet positions that carry the IPv4 address: the four from
    /// `length / 8` on, skipping the u octet.
    fn ipv4_octets(&self) -> [usize; 4] {
        let start = usize::from(self.length / 8);
        let mut positions = (start..16).filter(|octet| *octet != U_OCTET);
        let mut take = || {
            positions
                .next()
                .expect("every RFC 6052 length leaves four octets")
        };
        [take(), take(), take(), take()]
    }

    /// The IPv4 address `address` embeds under this prefix (RFC 6052 §2.2).
    pub fn embedded(&self, address: Ipv6Addr) -> Ipv4Addr {
        assert!(self.contains(address), "{address} is not under {self:?}");
        let octets = address.octets();
        let [a, b, c, d] = self.ipv4_octets();
        Ipv4Addr::new(octets[a], octets[b], octets[c], octets[d])
    }

    /// The IPv6 address this prefix gives `ipv4` (u octet and suffix zero).
    pub fn synthesize(&self, ipv4: Ipv4Addr) -> Ipv6Addr {
        let mut octets = self.prefix.octets();
        for (position, octet) in self.ipv4_octets().into_iter().zip(ipv4.octets()) {
            octets[position] = octet;
        }
        Ipv6Addr::from(octets)
    }
}

/// `address` with every bit past `length` cleared.
fn masked(address: Ipv6Addr, length: u8) -> Ipv6Addr {
    let mask = u128::MAX.checked_shl(128 - u32::from(length)).unwrap_or(0);
    Ipv6Addr::from(u128::from(address) & mask)
}

/// The prefixes RFC 7050 discovery reveals: for each AAAA answer for
/// [`DISCOVERY_NAME`], every RFC 6052 length at which it embeds a
/// well-known IPv4 address of that name. No answers, no prefixes.
pub fn discovered_prefixes(answers: &[Ipv6Addr]) -> Vec<Nat64Prefix> {
    let mut found = Vec::new();
    for answer in answers {
        for length in LENGTHS {
            let candidate = Nat64Prefix::new(*answer, length).expect("an RFC 6052 length");
            // Below /96 the u octet is outside the prefix and must be zero.
            let u_octet_clear = length == 96 || answer.octets()[U_OCTET] == 0;
            let matches = WELL_KNOWN_IPV4.contains(&candidate.embedded(*answer)) && u_octet_clear;
            if matches && !found.contains(&candidate) {
                found.push(candidate);
            }
        }
    }
    found
}

/// The IPv4 address a discovered prefix translates `address` to, if any
/// discovered prefix holds it.
pub fn translated(address: Ipv6Addr, prefixes: &[Nat64Prefix]) -> Option<Ipv4Addr> {
    prefixes
        .iter()
        .find(|prefix| prefix.contains(address))
        .map(|prefix| prefix.embedded(address))
}

#[cfg(test)]
#[path = "nat64_prefix_tests.rs"]
mod tests;
