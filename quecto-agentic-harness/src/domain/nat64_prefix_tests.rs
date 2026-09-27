use super::*;

fn v6(text: &str) -> Ipv6Addr {
    text.parse().expect("an IPv6 address")
}

/// RFC 6052 §2.4, Table 1: 192.0.2.33 under each prefix length.
const RFC_6052_EXAMPLES: [(&str, u8, &str); 6] = [
    ("2001:db8::", 32, "2001:db8:c000:221::"),
    ("2001:db8:100::", 40, "2001:db8:1c0:2:21::"),
    ("2001:db8:122::", 48, "2001:db8:122:c000:2:2100::"),
    ("2001:db8:122:300::", 56, "2001:db8:122:3c0:0:221::"),
    ("2001:db8:122:344::", 64, "2001:db8:122:344:c0:2:2100:0"),
    ("2001:db8:122:344::", 96, "2001:db8:122:344::c000:221"),
];

#[test]
fn every_rfc_6052_example_extracts_and_synthesizes_192_0_2_33() {
    let ipv4 = Ipv4Addr::new(192, 0, 2, 33);
    for (prefix, length, synthesized) in RFC_6052_EXAMPLES {
        let prefix = Nat64Prefix::new(v6(prefix), length).unwrap();
        assert!(prefix.contains(v6(synthesized)), "/{length}");
        assert_eq!(prefix.embedded(v6(synthesized)), ipv4, "/{length}");
        assert_eq!(prefix.synthesize(ipv4), v6(synthesized), "/{length}");
    }
}

#[test]
fn the_u_octet_is_skipped_below_96_and_never_carries_ipv4_bits() {
    // /64: the IPv4 address is in octets 9-12; octet 8 (the u octet) is not
    // read even when set.
    let prefix = Nat64Prefix::new(v6("2001:db8:122:344::"), 64).unwrap();
    assert_eq!(
        prefix.embedded(v6("2001:db8:122:344:ffc0:2:2100:0")),
        Ipv4Addr::new(192, 0, 2, 33)
    );
    // /32: octets 4-7 then, past the u octet, nothing more.
    let prefix = Nat64Prefix::new(v6("2001:db8::"), 32).unwrap();
    assert_eq!(
        prefix.embedded(v6("2001:db8:a00:1:ff00::")),
        Ipv4Addr::new(10, 0, 0, 1)
    );
    // /40: three octets, the u octet, then one more.
    let prefix = Nat64Prefix::new(v6("2001:db8:100::"), 40).unwrap();
    assert_eq!(
        prefix.embedded(v6("2001:db8:10a:0:1::")),
        Ipv4Addr::new(10, 0, 0, 1)
    );
}

#[test]
fn only_rfc_6052_lengths_make_a_prefix_and_host_bits_are_cleared() {
    for length in [0, 16, 31, 33, 63, 65, 95, 97, 128] {
        assert_eq!(
            Nat64Prefix::new(v6("2001:db8::"), length),
            None,
            "/{length}"
        );
    }
    let prefix = Nat64Prefix::new(v6("2001:db8:122:344::1:2"), 64).unwrap();
    assert_eq!(prefix.prefix(), v6("2001:db8:122:344::"));
    assert_eq!(prefix.length(), 64);
    assert!(!prefix.contains(v6("2001:db8:122:345::1")));
}

#[test]
fn discovery_finds_each_prefix_from_its_ipv4only_arpa_answers() {
    for (prefix, length, _) in RFC_6052_EXAMPLES {
        let expected = Nat64Prefix::new(v6(prefix), length).unwrap();
        let answers = [
            expected.synthesize(Ipv4Addr::new(192, 0, 0, 170)),
            expected.synthesize(Ipv4Addr::new(192, 0, 0, 171)),
        ];
        assert_eq!(discovered_prefixes(&answers), vec![expected], "/{length}");
    }
    // The well-known prefix is found like any other.
    assert_eq!(
        discovered_prefixes(&[v6("64:ff9b::c000:aa")]),
        vec![Nat64Prefix::new(v6("64:ff9b::"), 96).unwrap()]
    );
}

#[test]
fn no_discovery_answers_or_unrelated_answers_mean_no_prefix() {
    assert_eq!(discovered_prefixes(&[]), Vec::new());
    for unrelated in [
        "2001:db8::1",
        "2001:db8:122:344::c000:2ab",
        "::1",
        "2001:db8:c000:aa:ff00::",
    ] {
        assert_eq!(
            discovered_prefixes(&[v6(unrelated)]),
            Vec::new(),
            "{unrelated}"
        );
    }
}

#[test]
fn translation_uses_only_the_prefix_that_holds_the_address() {
    let slash96 = Nat64Prefix::new(v6("2001:4860:1234:5678:9abc:de00::"), 96).unwrap();
    let slash64 = Nat64Prefix::new(v6("2001:db8:64:64::"), 64).unwrap();
    let prefixes = [slash96, slash64];
    assert_eq!(
        translated(v6("2001:4860:1234:5678:9abc:de00:a00:1"), &prefixes),
        Some(Ipv4Addr::new(10, 0, 0, 1))
    );
    assert_eq!(
        translated(v6("2001:db8:64:64:a9:fea9:fe00:0"), &prefixes),
        Some(Ipv4Addr::new(169, 254, 169, 254))
    );
    assert_eq!(translated(v6("2001:4860::1"), &prefixes), None);
    assert_eq!(
        translated(v6("2001:4860:1234:5678:9abc:de00:a00:1"), &[]),
        None
    );
}
