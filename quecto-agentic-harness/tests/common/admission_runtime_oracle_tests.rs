//! Counterexamples for the same runtime observation assertions.
use super::*;

#[test]
fn runtime_oracles_accept_control_observations_and_reject_counterexamples() {
    // These are result/trace perturbations, NOT production mutation evidence.
    // Keep controls: catch_unwind alone could hide an always-panicking oracle.
    let state = GroupSnapshot {
        active: 1,
        queued: 1,
        uncertain: 0,
        cooldown_until: 91,
        unavailable: false,
        observed_at: 90,
    };
    let trace = vec![("retained-model".to_owned(), "Bearer live-secret".to_owned())];
    let expected = [("retained-model", "Bearer live-secret")];
    assert_wire_trace(&trace, &expected);
    assert_budget(state, (1, 1, 91));
    assert_retained_publication("restart required", Some(2), 2, 2, 2);
    assert_blocked(true);
    assert_content(Some("runtime-ok"));
    let reject = |name: &str, oracle: &dyn Fn()| {
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(oracle)).is_err(),
            "oracle failed to detect counterexample: {name}"
        );
    };
    reject("unexpected pre-deadline send", &|| {
        assert_wire_trace(&trace, &[])
    });
    reject("missing distinct-group progress", &|| {
        assert_wire_trace(&[], &expected)
    });
    reject("duplicate raw send", &|| {
        assert_wire_trace(&[trace[0].clone(), trace[0].clone()], &expected)
    });
    reject("changed model", &|| {
        assert_wire_trace(&[("wrong-model".into(), trace[0].1.clone())], &expected)
    });
    reject("rejected candidate credential published", &|| {
        assert_wire_trace(
            &[(trace[0].0.clone(), "Bearer must-not-publish".into())],
            &expected,
        )
    });
    reject("lost active permit", &|| {
        assert_budget(GroupSnapshot { active: 0, ..state }, (1, 1, 91))
    });
    reject("duplicate occupancy", &|| {
        assert_budget(GroupSnapshot { active: 2, ..state }, (1, 1, 91))
    });
    reject("bypass retained shared queue", &|| {
        assert_budget(GroupSnapshot { queued: 0, ..state }, (1, 1, 91))
    });
    reject("wrong distinct-group queue", &|| {
        assert_budget(
            GroupSnapshot {
                active: 0,
                queued: 1,
                uncertain: 0,
                cooldown_until: 0,
                ..state
            },
            (0, 0, 0),
        )
    });
    reject("cleared cooldown", &|| {
        assert_budget(
            GroupSnapshot {
                cooldown_until: 0,
                ..state
            },
            (1, 1, 91),
        )
    });
    reject("shortened boundary", &|| {
        assert_budget(
            GroupSnapshot {
                cooldown_until: 90,
                ..state
            },
            (1, 1, 91),
        )
    });
    reject("reanchored boundary", &|| {
        assert_budget(
            GroupSnapshot {
                cooldown_until: 92,
                ..state
            },
            (1, 1, 91),
        )
    });
    reject("invisible restart requirement", &|| {
        assert_retained_publication("invalid", Some(2), 2, 2, 2)
    });
    reject("missing retained snapshot", &|| {
        assert_retained_publication("restart", None, 2, 2, 2)
    });
    reject("wrong retained snapshot", &|| {
        assert_retained_publication("restart", Some(3), 2, 2, 2)
    });
    reject("runtime candidate published", &|| {
        assert_retained_publication("restart", Some(2), 3, 2, 2)
    });
    reject("catalogue candidate published", &|| {
        assert_retained_publication("restart", Some(2), 2, 3, 2)
    });
    reject("early dispatch", &|| assert_blocked(false));
    reject("missing response", &|| assert_content(None));
    reject("wrong response", &|| assert_content(Some("wrong")));
}
