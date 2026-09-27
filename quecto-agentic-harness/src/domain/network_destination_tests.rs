use super::*;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

fn ip(text: &str) -> IpAddr {
    text.parse().expect("test address parses")
}

fn refusal(text: &str) -> String {
    authorize_destination(ip(text)).expect_err(text).to_string()
}

#[test]
fn globally_routable_ipv4_unicast_is_public() {
    for public in [
        "1.1.1.1",
        "8.8.8.8",
        "93.184.216.34",
        "223.255.255.254",
        // Just outside each special block.
        "9.255.255.255",
        "11.0.0.0",
        "100.63.255.255",
        "100.128.0.0",
        "126.255.255.255",
        "128.0.0.0",
        "169.253.255.255",
        "169.255.0.0",
        "172.15.255.255",
        "172.32.0.0",
        "191.255.255.255",
        "192.0.1.0",
        "192.0.3.0",
        "192.88.98.255",
        "192.88.100.0",
        "192.167.255.255",
        "192.169.0.0",
        "198.17.255.255",
        "198.20.0.0",
        "198.51.99.255",
        "198.51.101.0",
        "203.0.112.255",
        "203.0.114.0",
    ] {
        assert!(is_public_destination(ip(public)), "{public} is public");
        assert_eq!(authorize_destination(ip(public)), Ok(()), "{public}");
    }
}

#[test]
fn every_non_global_ipv4_block_is_refused_at_both_ends() {
    for (first, last) in [
        ("0.0.0.0", "0.255.255.255"),           // this network
        ("10.0.0.0", "10.255.255.255"),         // private
        ("100.64.0.0", "100.127.255.255"),      // CGNAT shared space
        ("127.0.0.0", "127.255.255.255"),       // loopback
        ("169.254.0.0", "169.254.255.255"),     // link-local, cloud metadata
        ("172.16.0.0", "172.31.255.255"),       // private
        ("192.0.0.0", "192.0.0.255"),           // IETF protocol assignments
        ("192.0.2.0", "192.0.2.255"),           // TEST-NET-1
        ("192.88.99.0", "192.88.99.255"),       // 6to4 relay anycast
        ("192.168.0.0", "192.168.255.255"),     // private
        ("198.18.0.0", "198.19.255.255"),       // benchmarking
        ("198.51.100.0", "198.51.100.255"),     // TEST-NET-2
        ("203.0.113.0", "203.0.113.255"),       // TEST-NET-3
        ("224.0.0.0", "239.255.255.255"),       // multicast
        ("240.0.0.0", "255.255.255.254"),       // reserved
        ("255.255.255.255", "255.255.255.255"), // broadcast
    ] {
        for address in [first, last] {
            assert!(!is_public_destination(ip(address)), "{address} is refused");
        }
    }
    assert!(!is_public_destination(ip("169.254.169.254")));
}

#[test]
fn allocated_global_ipv6_unicast_is_public() {
    for public in [
        "2001:4860:4860::8888", // Google DNS
        "2606:4700:4700::1111", // Cloudflare DNS
        "2a00:1450:4001:81c::200e",
        "2400:cb00::1",
        "2800:3f0::1",
        "2c0f:fb50::1",
        "2003::1",
        "2610::1",
        "2620:0:ccc::2",
        "2630::1",
        "2a10::1",
        "2001:200::1", // first allocation after the IETF block
    ] {
        assert!(is_public_destination(ip(public)), "{public} is public");
    }
}

#[test]
fn non_global_ipv6_is_refused() {
    for refused in [
        "::",               // unspecified
        "::1",              // loopback
        "fc00::1",          // unique local
        "fdff:ffff::1",     // unique local, upper end
        "fe80::1",          // link-local
        "febf::1",          // link-local, upper end
        "fec0::1",          // deprecated site-local
        "ff02::1",          // multicast
        "ff0e::1",          // global-scope multicast is still multicast
        "2001:db8::1",      // documentation
        "2001:db8:ffff::1", // documentation, upper end
        "3fff::1",          // documentation (RFC 9637)
        "100::1",           // discard-only
        "2001:2::1",        // benchmarking, inside the IETF block
        "2001:10::1",       // ORCHID, inside the IETF block
        "2001:1ff::1",      // IETF block, upper end
        "64:ff9b:1::1",     // local-use NAT64
        "3000::1",          // unallocated global unicast hole
        "3ffe::1",          // retired 6bone
        "2004::1",          // unallocated
        "4000::1",          // outside global unicast
    ] {
        assert!(!is_public_destination(ip(refused)), "{refused} is refused");
    }
}

#[test]
fn ipv4_mapped_addresses_follow_the_ipv4_rules() {
    assert!(!is_public_destination(ip("::ffff:127.0.0.1")));
    assert!(!is_public_destination(ip("::ffff:a9fe:a9fe")));
    assert!(!is_public_destination(ip("::ffff:10.1.2.3")));
    assert!(!is_public_destination(ip("::ffff:0.0.0.0")));
    assert!(is_public_destination(ip("::ffff:8.8.8.8")));
}

#[test]
fn ipv4_compatible_addresses_are_never_public() {
    for refused in ["::127.0.0.1", "::a9fe:a9fe", "::8.8.8.8", "::0.0.0.2"] {
        assert!(!is_public_destination(ip(refused)), "{refused}");
    }
}

#[test]
fn nat64_well_known_prefix_follows_the_ipv4_rules() {
    assert!(!is_public_destination(ip("64:ff9b::127.0.0.1")));
    assert!(!is_public_destination(ip("64:ff9b::a9fe:a9fe")));
    assert!(!is_public_destination(ip("64:ff9b::192.168.0.1")));
    assert!(is_public_destination(ip("64:ff9b::8.8.8.8")));
    // Only the /96: the rest of 64:ff9b::/32 is not NAT64.
    assert!(!is_public_destination(ip("64:ff9b:0:1::8.8.8.8")));
}

#[test]
fn sixtofour_follows_the_ipv4_rules_of_its_embedded_address() {
    assert!(!is_public_destination(ip("2002:7f00:1::1"))); // 127.0.0.1
    assert!(!is_public_destination(ip("2002:a9fe:a9fe::1"))); // metadata
    assert!(!is_public_destination(ip("2002:0a00:0001::1"))); // 10.0.0.1
    assert!(is_public_destination(ip("2002:0808:0808::1"))); // 8.8.8.8
}

#[test]
fn teredo_needs_both_embedded_ipv4_addresses_public() {
    // Server 65.54.227.120, client 192.0.2.45 obfuscated (documentation).
    let documentation_client = "2001:0:4136:e378:8000:63bf:3fff:fdd2";
    assert!(!is_public_destination(ip(documentation_client)));
    // Server 8.8.8.8, client 127.0.0.1 obfuscated.
    assert!(!is_public_destination(ip("2001:0:808:808::80ff:fffe")));
    // Server 127.0.0.1, client 8.8.8.8 obfuscated.
    assert!(!is_public_destination(ip("2001:0:7f00:1::f7f7:f7f7")));
    // Server 8.8.8.8, client 1.1.1.1 obfuscated.
    assert!(is_public_destination(ip("2001:0:808:808::fefe:fefe")));
}

#[test]
fn refusals_name_the_address_and_any_embedded_ipv4_it_stands_for() {
    assert_eq!(refusal("127.0.0.1"), "127.0.0.1 is not a public address");
    assert_eq!(refusal("fe80::1"), "fe80::1 is not a public address");
    assert_eq!(
        refusal("::ffff:a9fe:a9fe"),
        "169.254.169.254 (via ::ffff:169.254.169.254) is not a public address"
    );
    assert_eq!(
        refusal("64:ff9b::7f00:1"),
        "127.0.0.1 (via 64:ff9b::7f00:1) is not a public address"
    );
    assert_eq!(
        refusal("2002:a9fe:a9fe::1"),
        "169.254.169.254 (via 2002:a9fe:a9fe::1) is not a public address"
    );
    assert_eq!(
        refusal("2001:0:808:808::80ff:fffe"),
        "127.0.0.1 (via 2001:0:808:808::80ff:fffe) is not a public address"
    );
    assert_eq!(
        refusal("2001:0:7f00:1::f7f7:f7f7"),
        "127.0.0.1 (via 2001:0:7f00:1::f7f7:f7f7) is not a public address"
    );
    let refused = authorize_destination(ip("::ffff:127.0.0.1")).unwrap_err();
    assert_eq!(refused.address(), ip("::ffff:127.0.0.1"));
    assert_eq!(refused.embedded(), Some(Ipv4Addr::LOCALHOST));
    assert_eq!(
        authorize_destination(IpAddr::V6(Ipv6Addr::LOCALHOST))
            .unwrap_err()
            .embedded(),
        None
    );
}
