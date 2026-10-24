use super::*;

#[test]
fn loopback_and_tailscale_addresses_are_allowed() {
    for allowed in [
        "127.0.0.1:7979",
        "127.0.0.1:0",
        "127.8.9.10:7979",
        "[::1]:7979",
        "100.64.0.0:7979",
        "100.100.12.34:7979",
        "100.127.255.255:7979",
        "[fd7a:115c:a1e0::1]:7979",
        "[fd7a:115c:a1e0:ab12:4843:cd96:6258:b240]:7979",
    ] {
        let address = ListenAddress::parse(allowed).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(address.socket_addr(), allowed.parse().unwrap());
    }
}

/// B5 red: `0.0.0.0` and `::` (every interface) and any address off the
/// allowlist are refused, saying what is allowed.
#[test]
fn every_other_address_is_refused_saying_what_is_allowed() {
    for refused in [
        "0.0.0.0:7979",
        "[::]:7979",
        "10.0.0.1:7979",
        "192.168.1.20:7979",
        "100.63.255.255:7979",
        "100.128.0.1:7979",
        "8.8.8.8:7979",
        "[::ffff:127.0.0.1]:7979",
        "[fd7a:115c:a1e1::1]:7979",
        "[fe80::1]:7979",
    ] {
        let why = ListenAddress::parse(refused).unwrap_err();
        assert!(why.contains("is refused"), "{refused}: {why}");
        assert!(why.contains("100.64.0.0/10"), "{refused}: {why}");
    }
}

#[test]
fn text_that_is_no_address_and_port_is_refused() {
    for refused in [
        "",
        "127.0.0.1",
        "localhost:7979",
        "::1:7979",
        "127.0.0.1:99999",
    ] {
        let why = ListenAddress::parse(refused).unwrap_err();
        assert!(why.contains("is not an <ip>:<port>"), "{refused}: {why}");
    }
}
