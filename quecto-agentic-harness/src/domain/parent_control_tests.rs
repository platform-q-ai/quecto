use super::*;

fn cap(byte: u8) -> ParentControlCapability {
    ParentControlCapability::from_random_bytes(&[byte; 32])
}

fn credential(generation: u64, byte: u8) -> ParentControlCredential {
    ParentControlCredential {
        generation: LaunchGeneration::new(generation),
        capability: cap(byte),
    }
}

#[test]
fn capability_parse_is_an_affirmative_allowlist() {
    let good = "a".repeat(64);
    assert!(ParentControlCapability::parse(&good).is_ok());
    assert_eq!(
        ParentControlCapability::parse(&"a".repeat(63)),
        Err(CapabilityFormatError::WrongLength { actual: 63 })
    );
    assert_eq!(
        ParentControlCapability::parse(&"A".repeat(64)),
        Err(CapabilityFormatError::NotLowercaseHex)
    );
    assert_eq!(
        ParentControlCapability::parse(&"g".repeat(64)),
        Err(CapabilityFormatError::NotLowercaseHex)
    );
    assert_eq!(
        ParentControlCapability::parse(""),
        Err(CapabilityFormatError::WrongLength { actual: 0 })
    );
    assert!(
        CapabilityFormatError::WrongLength { actual: 1 }
            .to_string()
            .contains("64 hex")
    );
    assert!(
        CapabilityFormatError::NotLowercaseHex
            .to_string()
            .contains("lowercase")
    );
}

#[test]
fn capability_renders_redacted_everywhere_but_expose() {
    let capability = cap(0xab);
    assert_eq!(
        format!("{capability:?}"),
        "ParentControlCapability(<redacted>)"
    );
    assert_eq!(capability.to_string(), "<redacted>");
    assert_eq!(capability.expose(), "ab".repeat(32));
    assert_eq!(capability.expose().len(), CAPABILITY_HEX_LEN);
    assert!(ParentControlCapability::parse(capability.expose()).is_ok());
}

#[test]
fn fail_closed_matrix() {
    // (binding, generation presented, capability presented, expected)
    let cases: Vec<(ParentControlBinding, u64, u8, BindRejection)> = vec![
        (
            ParentControlBinding::unlaunched(),
            1,
            1,
            BindRejection::NotALaunchedChild,
        ),
        (
            ParentControlBinding::launched(credential(3, 1)),
            2,
            1,
            BindRejection::WrongGeneration {
                expected: LaunchGeneration::new(3),
                presented: LaunchGeneration::new(2),
            },
        ),
        (
            ParentControlBinding::launched(credential(3, 1)),
            3,
            2,
            BindRejection::Mismatch,
        ),
    ];
    for (mut binding, generation, byte, expected) in cases {
        let outcome = binding.present(LaunchGeneration::new(generation), &cap(byte));
        assert_eq!(outcome, Err(expected));
        assert_eq!(binding.state(), BindingState::Unbound);
        assert_eq!(
            binding.connection_closed(false),
            ConnectionLoss::OrdinaryClient,
            "a refused presenter is an ordinary loss"
        );
    }
}

#[test]
fn exactly_one_presentation_binds_and_replays_are_refused() {
    let mut binding = ParentControlBinding::launched(credential(7, 9));
    assert!(binding.is_launched_child());
    assert_eq!(binding.present(LaunchGeneration::new(7), &cap(9)), Ok(()));
    assert_eq!(binding.state(), BindingState::Bound);
    // The same, correct capability presented again is a replay.
    assert_eq!(
        binding.present(LaunchGeneration::new(7), &cap(9)),
        Err(BindRejection::AlreadyBound)
    );
    // A wrong capability while bound is still just refused.
    assert_eq!(
        binding.present(LaunchGeneration::new(7), &cap(1)),
        Err(BindRejection::AlreadyBound)
    );
    assert_eq!(binding.state(), BindingState::Bound);
}

#[test]
fn only_the_bound_connection_loss_counts_and_only_once() {
    let mut binding = ParentControlBinding::launched(credential(1, 4));
    // Ordinary clients closing before, during and after the binding never
    // count, even the last one.
    assert_eq!(
        binding.connection_closed(false),
        ConnectionLoss::OrdinaryClient
    );
    binding
        .present(LaunchGeneration::new(1), &cap(4))
        .expect("binds");
    assert_eq!(
        binding.connection_closed(false),
        ConnectionLoss::OrdinaryClient
    );
    assert_eq!(binding.state(), BindingState::Bound);
    assert_eq!(
        binding.connection_closed(true),
        ConnectionLoss::BoundParentLost
    );
    assert_eq!(binding.state(), BindingState::Lost);
    // A second report of the bound loss is inert, and nothing can bind again.
    assert_eq!(
        binding.connection_closed(true),
        ConnectionLoss::OrdinaryClient
    );
    assert_eq!(
        binding.present(LaunchGeneration::new(1), &cap(4)),
        Err(BindRejection::AlreadyLost)
    );
}

#[test]
fn an_unbound_binding_never_reports_a_parent_loss() {
    let mut binding = ParentControlBinding::launched(credential(1, 4));
    // A caller lying about having bound cannot manufacture a loss.
    assert_eq!(
        binding.connection_closed(true),
        ConnectionLoss::OrdinaryClient
    );
    assert_eq!(binding.state(), BindingState::Unbound);
    let mut top_level = ParentControlBinding::unlaunched();
    assert!(!top_level.is_launched_child());
    assert_eq!(
        top_level.connection_closed(true),
        ConnectionLoss::OrdinaryClient
    );
}

#[test]
fn rejections_display_without_the_secret() {
    for rejection in [
        BindRejection::NotALaunchedChild,
        BindRejection::WrongGeneration {
            expected: LaunchGeneration::new(1),
            presented: LaunchGeneration::new(2),
        },
        BindRejection::Mismatch,
        BindRejection::AlreadyBound,
        BindRejection::AlreadyLost,
    ] {
        let text = rejection.to_string();
        assert!(!text.is_empty());
        assert!(!text.contains("aaaa"));
    }
    let credential = credential(1, 0xaa);
    assert!(!format!("{credential:?}").contains("aaaa"));
}

#[test]
fn equality_is_constant_time_and_still_exact() {
    assert_eq!(cap(1), cap(1));
    assert_ne!(cap(1), cap(2));
    let parsed = ParentControlCapability::parse(&"ab".repeat(32)).unwrap();
    assert_eq!(parsed, cap(0xab));
}

#[test]
fn only_an_unbound_launched_binding_expires() {
    let mut launched = ParentControlBinding::launched(credential(1, 1));
    assert!(launched.expire_unbound());
    assert_eq!(launched.state(), BindingState::Lost);
    assert!(!launched.expire_unbound(), "expiry is reported once");
    assert_eq!(
        launched.present(LaunchGeneration::new(1), &cap(1)),
        Err(BindRejection::AlreadyLost),
        "a late parent is refused"
    );
    let mut bound = ParentControlBinding::launched(credential(1, 1));
    bound.present(LaunchGeneration::new(1), &cap(1)).unwrap();
    assert!(!bound.expire_unbound());
    assert_eq!(bound.state(), BindingState::Bound);
    let mut top_level = ParentControlBinding::unlaunched();
    assert!(!top_level.expire_unbound());
}
