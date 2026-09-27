//! #2198: every command-policy rule explains itself — what it refuses, why,
//! and what to do instead — and keeps its stable id for the logs.
use super::policy_rule::PolicyRule;

/// Every rule the structural denylist can report, with the id it is logged
/// under. The fallback scan is covered separately: its pattern is data.
const STRUCTURAL: &[(PolicyRule, &str)] = &[
    (PolicyRule::GlobCommandName, "glob-command-name"),
    (PolicyRule::BlockDeviceWrite, "block-device-write"),
    (PolicyRule::RmRoot, "rm-root"),
    (PolicyRule::RmNoPreserveRoot, "rm-no-preserve-root"),
    (PolicyRule::RmProtectedDir, "rm-protected-dir"),
    (PolicyRule::Mkfs, "mkfs"),
    (PolicyRule::DdDeviceSource, "dd-device-source"),
    (PolicyRule::DdBlockDevice, "dd-block-device"),
    (PolicyRule::PowerState, "power-state"),
    (PolicyRule::ChmodRoot, "chmod-root"),
    (PolicyRule::ChownRoot, "chown-root"),
    (PolicyRule::FetchToShell, "fetch-to-shell"),
    (PolicyRule::ForkBomb, "fork-bomb"),
];

/// Where a rule sits in [`STRUCTURAL`]. The match is exhaustive: a new rule
/// does not compile until it is given a place here, and the test below
/// fails until it is listed there.
fn position(rule: PolicyRule) -> Option<usize> {
    match rule {
        PolicyRule::GlobCommandName => Some(0),
        PolicyRule::BlockDeviceWrite => Some(1),
        PolicyRule::RmRoot => Some(2),
        PolicyRule::RmNoPreserveRoot => Some(3),
        PolicyRule::RmProtectedDir => Some(4),
        PolicyRule::Mkfs => Some(5),
        PolicyRule::DdDeviceSource => Some(6),
        PolicyRule::DdBlockDevice => Some(7),
        PolicyRule::PowerState => Some(8),
        PolicyRule::ChmodRoot => Some(9),
        PolicyRule::ChownRoot => Some(10),
        PolicyRule::FetchToShell => Some(11),
        PolicyRule::ForkBomb => Some(12),
        PolicyRule::Fallback(_) => None,
    }
}

/// The highest place [`position`] gives.
const LAST_POSITION: usize = 12;

#[test]
fn structural_lists_every_rule_once() {
    assert_eq!(STRUCTURAL.len(), LAST_POSITION + 1);
    for (index, (rule, _)) in STRUCTURAL.iter().enumerate() {
        assert_eq!(position(*rule), Some(index), "{}", rule.id());
    }
    assert_eq!(position(PolicyRule::Fallback("reboot")), None);
}

#[test]
fn each_rule_keeps_its_stable_id() {
    for (rule, id) in STRUCTURAL {
        assert_eq!(rule.id(), *id);
    }
    assert_eq!(PolicyRule::Fallback("rm -rf /").id(), "fallback-scan");
}

#[test]
fn each_rule_has_a_reason_and_a_way_forward() {
    let rules = STRUCTURAL
        .iter()
        .map(|(rule, _)| *rule)
        .chain([PolicyRule::Fallback("reboot")]);
    for rule in rules {
        let reason = rule.reason();
        let instead = rule.instead();
        let closing = rule.closing();
        assert!(
            closing.contains("tell the user"),
            "{}: {closing}",
            rule.id()
        );
        assert!(
            closing.contains("another way is not allowed"),
            "{}: {closing}",
            rule.id()
        );
        // No way forward names a place to hide the command (#2198 review).
        assert!(!instead.contains("in a file"), "{}: {instead}", rule.id());
        assert!(reason.trim().len() > 20, "{}: reason {reason:?}", rule.id());
        assert!(
            instead.trim().len() > 10,
            "{}: instead {instead:?}",
            rule.id()
        );
        // The id is for the logs; the words must say more than the id.
        assert_ne!(reason.trim(), rule.id());
        assert!(
            !reason.ends_with('.'),
            "{}: the caller punctuates",
            rule.id()
        );
        assert!(
            !instead.ends_with('.'),
            "{}: the caller punctuates",
            rule.id()
        );
    }
}

/// Reasons are distinct: a model reading one learns what this rule is about.
#[test]
fn no_two_rules_share_a_reason() {
    let mut reasons: Vec<String> = STRUCTURAL.iter().map(|(rule, _)| rule.reason()).collect();
    reasons.sort();
    let before = reasons.len();
    reasons.dedup();
    assert_eq!(reasons.len(), before);
}

#[test]
fn a_glob_in_the_program_name_says_to_name_it_literally() {
    let rule = PolicyRule::GlobCommandName;
    assert!(rule.reason().contains("glob"), "{}", rule.reason());
    assert!(rule.instead().contains("literally"), "{}", rule.instead());
}

/// #2198 review: a blanket rule names the programs it covers, not the
/// effect (other formatters and `loginctl` are not covered).
#[test]
fn blanket_rules_name_their_programs() {
    let mkfs = PolicyRule::Mkfs.reason();
    assert!(
        mkfs.starts_with("`mkfs*`, `mke2fs` and `mkswap` do not run here in any form"),
        "{mkfs}"
    );
    let power = PolicyRule::PowerState.reason();
    assert!(
        power
            .starts_with("`shutdown`, `reboot`, `halt` and `poweroff` do not run here in any form"),
        "{power}"
    );
    for reason in [mkfs, power] {
        assert!(!reason.contains("refused in any form"), "{reason}");
    }
}

/// #2198 review: `dd` from a device source points at the ways to fill a
/// regular file, which the policy allows.
#[test]
fn dd_from_a_device_offers_a_way_to_fill_a_file() {
    let rule = PolicyRule::DdDeviceSource;
    assert!(rule.reason().contains("wipe a disk"), "{}", rule.reason());
    assert_eq!(
        rule.instead(),
        "to fill a regular file, use `head -c <size> /dev/zero > file` (or `/dev/urandom`), or \
         `truncate -s <size> file`"
    );
    for suggested in [
        "head -c 1024 /dev/zero > file",
        "head -c 1024 /dev/urandom > file",
        "truncate -s 1024 file",
    ] {
        assert!(super::denylist::check(suggested).is_ok(), "{suggested}");
    }
}

/// Rules that protect data say what they would destroy.
#[test]
fn delete_and_ownership_rules_say_what_they_would_destroy() {
    for rule in [
        PolicyRule::RmRoot,
        PolicyRule::RmProtectedDir,
        PolicyRule::ChmodRoot,
        PolicyRule::ChownRoot,
    ] {
        let reason = rule.reason();
        assert!(reason.contains("would"), "{}: {reason}", rule.id());
        assert!(!reason.ends_with("is refused"), "{}: {reason}", rule.id());
    }
}

#[test]
fn a_fallback_names_the_text_it_found_and_splits_data_from_commands() {
    let rule = PolicyRule::Fallback("reboot");
    assert!(rule.reason().contains("`reboot`"), "{}", rule.reason());
    let instead = rule.instead();
    assert!(
        instead.starts_with(
            "if the words are only data, write the `$…` part (program or script) literally"
        ),
        "{instead}"
    );
    assert!(
        instead.contains("if the command would really run it, stop and tell the user"),
        "{instead}"
    );
}

/// Rules that refuse an effect close on it; rules that refuse an unreadable
/// form say the readable form is checked again, not waved through.
#[test]
fn the_closing_fits_the_way_forward() {
    assert_eq!(
        PolicyRule::Mkfs.closing(),
        "Getting the same effect another way is not allowed; if the task needs it, tell the user."
    );
    for rule in [PolicyRule::GlobCommandName, PolicyRule::Fallback("reboot")] {
        assert!(
            rule.closing().contains("checked again"),
            "{}",
            rule.closing()
        );
    }
}
