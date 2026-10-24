//! Where a board server listens over TCP (epic #2705 B5): an allowlist of
//! loopback and Tailscale's ranges. Every other address is refused, the
//! unspecified ones (`0.0.0.0`, `::`) first among them.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// The addresses a board server may listen on, as its refusals say them.
pub const ALLOWED: &str = "loopback (127.0.0.0/8, ::1) or a Tailscale address \
                           (100.64.0.0/10, fd7a:115c:a1e0::/48)";

/// An `<ip>:<port>` on the allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenAddress(SocketAddr);

impl ListenAddress {
    /// `text` as an address to listen on.
    /// # Errors
    /// Not an `<ip>:<port>`, or an address off the allowlist.
    pub fn parse(text: &str) -> Result<Self, String> {
        let shown = text.escape_debug();
        let address: SocketAddr = text.parse().map_err(|_| {
            format!("'{shown}' is not an <ip>:<port> (IPv6 goes in brackets: [::1]:7979)")
        })?;
        match allowed(address.ip()) {
            true => Ok(Self(address)),
            false => Err(format!(
                "{shown} is refused: a board server listens only on {ALLOWED}"
            )),
        }
    }

    pub fn socket_addr(self) -> SocketAddr {
        self.0
    }
}

/// Loopback, or inside Tailscale's IPv4 or IPv6 range.
fn allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || tailscale_v4(ip),
        IpAddr::V6(ip) => ip == Ipv6Addr::LOCALHOST || tailscale_v6(ip),
    }
}

/// 100.64.0.0/10, Tailscale's carrier-grade NAT range.
fn tailscale_v4(ip: Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    first == 100 && second & 0b1100_0000 == 0b0100_0000
}

/// fd7a:115c:a1e0::/48, Tailscale's IPv6 range.
fn tailscale_v6(ip: Ipv6Addr) -> bool {
    matches!(ip.segments(), [0xfd7a, 0x115c, 0xa1e0, ..])
}

#[cfg(test)]
#[path = "listen_address_tests.rs"]
mod tests;
