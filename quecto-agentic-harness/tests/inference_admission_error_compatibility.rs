//! Local-review regression contracts: enabling admission must not rewrite the
//! existing leaf's observable errors, retry classification or assembled result.
#[path = "common/admission_error_fixture.rs"]
mod fixture;
use fixture::*;
use quecto::domain::provider_error::ProviderErrorClass;

async fn refused(leaf: Leaf, surface: Surface) {
    let (disabled, enabled, url) = compare_refused(leaf, surface).await;
    characterize_refused(&disabled, leaf, &url);
    assert_parity(&disabled, &enabled);
}
async fn truncated_429(leaf: Leaf) {
    let (disabled, enabled) = compare(
        leaf,
        Surface::Chat,
        Reply {
            status: 429,
            body: "partial".into(),
            missing_bytes: 32,
        },
    )
    .await;
    characterize_read_error(&disabled);
    assert_parity(&disabled, &enabled);
}
async fn oversized(leaf: Leaf) {
    // A multibyte codepoint straddles byte 4096: both display size and UTF-8
    // boundary preservation are compatibility contracts, not fixture policy.
    let body = format!("{}é{}", "x".repeat(4095), "tail".repeat(500));
    let (disabled, enabled) = compare(
        leaf,
        Surface::Incremental,
        Reply {
            status: 429,
            body,
            missing_bytes: 0,
        },
    )
    .await;
    characterize_oversized(&disabled, leaf);
    assert_parity(&disabled, &enabled);
}
async fn terminal_then_truncated(leaf: Leaf, surface: Surface) {
    let body = format!("{}\n\n{}", leaf.delta("visible"), leaf.terminal());
    let (disabled, enabled) = compare(
        leaf,
        surface,
        Reply {
            status: 200,
            body,
            missing_bytes: 32,
        },
    )
    .await;
    characterize_terminal(&disabled, leaf);
    assert_parity(&disabled, &enabled);
}
async fn unterminated(leaf: Leaf, surface: Surface) {
    let body = format!("{}\n\n{}", leaf.delta("first"), leaf.delta("last"));
    let (disabled, enabled) = compare(
        leaf,
        surface,
        Reply {
            status: 200,
            body,
            missing_bytes: 0,
        },
    )
    .await;
    characterize_unterminated(&disabled);
    assert_parity(&disabled, &enabled);
}
macro_rules! case {
    ($name:ident, $test:ident, $leaf:ident $(, $surface:ident)?) => {
        #[tokio::test]
        async fn $name() { $test(Leaf::$leaf $(, Surface::$surface)?).await; }
    };
}
case!(
    anthropic_chat_send_refusal_preserves_raw_display,
    refused,
    Anthropic,
    Chat
);
case!(
    anthropic_assembled_send_refusal_preserves_raw_display,
    refused,
    Anthropic,
    Assembled
);
case!(
    anthropic_incremental_send_refusal_preserves_raw_display,
    refused,
    Anthropic,
    Incremental
);
case!(
    responses_chat_send_refusal_preserves_codex_prefix,
    refused,
    Responses,
    Chat
);
case!(
    responses_incremental_send_refusal_preserves_codex_prefix,
    refused,
    Responses,
    Incremental
);
case!(
    oauth_chat_send_refusal_preserves_codex_prefix,
    refused,
    OAuth,
    Chat
);
case!(
    oauth_incremental_send_refusal_preserves_codex_prefix,
    refused,
    OAuth,
    Incremental
);
case!(
    openai_chat_truncated_429_preserves_body_read_error,
    truncated_429,
    OpenAi
);
case!(
    anthropic_chat_truncated_429_preserves_body_read_error,
    truncated_429,
    Anthropic
);
case!(
    openai_incremental_oversized_error_preserves_4kib_limit,
    oversized,
    OpenAi
);
case!(
    anthropic_incremental_oversized_error_preserves_4kib_limit,
    oversized,
    Anthropic
);
case!(
    responses_incremental_oversized_error_preserves_4kib_limit,
    oversized,
    Responses
);
case!(
    oauth_incremental_oversized_error_preserves_4kib_limit,
    oversized,
    OAuth
);
case!(
    anthropic_assembled_terminal_then_truncated_body_still_fails,
    terminal_then_truncated,
    Anthropic,
    Assembled
);
case!(
    responses_chat_terminal_then_truncated_body_still_fails,
    terminal_then_truncated,
    Responses,
    Chat
);
case!(
    responses_assembled_terminal_then_truncated_body_still_fails,
    terminal_then_truncated,
    Responses,
    Assembled
);
case!(
    oauth_chat_terminal_then_truncated_body_still_fails,
    terminal_then_truncated,
    OAuth,
    Chat
);
case!(
    oauth_assembled_terminal_then_truncated_body_still_fails,
    terminal_then_truncated,
    OAuth,
    Assembled
);
case!(
    anthropic_assembled_final_unterminated_valid_line_is_preserved,
    unterminated,
    Anthropic,
    Assembled
);
case!(
    responses_chat_final_unterminated_valid_line_is_preserved,
    unterminated,
    Responses,
    Chat
);
case!(
    responses_assembled_final_unterminated_valid_line_is_preserved,
    unterminated,
    Responses,
    Assembled
);
case!(
    oauth_chat_final_unterminated_valid_line_is_preserved,
    unterminated,
    OAuth,
    Chat
);
case!(
    oauth_assembled_final_unterminated_valid_line_is_preserved,
    unterminated,
    OAuth,
    Assembled
);

fn characterize_refused(disabled: &Observation, leaf: Leaf, url: &str) {
    match (disabled, leaf) {
        (Observation::Error(error), Leaf::Anthropic) => {
            assert_eq!(
                error.display,
                format!(
                    "provider error: HTTP error: error sending request for url ({url}/v1/messages)"
                )
            );
            assert_eq!(error.class, ProviderErrorClass::Unknown);
            assert!(!error.retryable);
        }
        (Observation::Events(events), Leaf::Anthropic) => {
            assert_eq!(events.len(), 1);
            let Event::Error { raw, classified } = &events[0] else {
                panic!("expected error")
            };
            assert_eq!(
                raw,
                &format!("HTTP error: error sending request for url ({url}/v1/messages)")
            );
            assert_eq!(classified.class, ProviderErrorClass::Unknown);
        }
        (Observation::Error(error), _) => {
            assert!(error.display.starts_with(
                "provider error: Codex request failed: error sending request for url ("
            ));
            assert_eq!(error.class, ProviderErrorClass::Unknown);
        }
        (Observation::Events(events), _) => {
            assert_eq!(events.len(), 1);
            let Event::Error { raw, classified } = &events[0] else {
                panic!("expected error")
            };
            assert!(raw.starts_with("Codex request failed: error sending request for url ("));
            assert_eq!(classified.class, ProviderErrorClass::Unknown);
        }
        _ => panic!("send refusal cannot succeed: {disabled:?}"),
    }
}

fn characterize_read_error(disabled: &Observation) {
    let Observation::Error(error) = disabled else {
        panic!("truncated response must fail")
    };
    assert_eq!(
        error.display,
        "provider error: failed to read response: error decoding response body"
    );
    assert_eq!(error.class, ProviderErrorClass::Unknown);
    assert_eq!(error.status, None);
}

fn characterize_oversized(disabled: &Observation, leaf: Leaf) {
    let Observation::Events(events) = disabled else {
        panic!("expected incremental events")
    };
    assert_eq!(events.len(), 1);
    let Event::Error { raw, classified } = &events[0] else {
        panic!("expected HTTP error")
    };
    assert_eq!(
        raw,
        &format!(
            "HTTP 429 from {}: {}... (truncated)",
            leaf.vendor(),
            "x".repeat(4095)
        )
    );
    assert_eq!(classified.class, ProviderErrorClass::RateLimit);
    assert_eq!(classified.status, Some(429));
}

fn characterize_terminal(disabled: &Observation, leaf: Leaf) {
    let Observation::Error(error) = disabled else {
        panic!("assembled legacy surface consumes complete HTTP body")
    };
    let prefix = if matches!(leaf, Leaf::Anthropic) {
        "failed to read stream"
    } else {
        "failed to read response"
    };
    assert_eq!(
        error.display,
        format!("provider error: {prefix}: error decoding response body")
    );
    assert_eq!(error.class, ProviderErrorClass::Unknown);
}

fn characterize_unterminated(disabled: &Observation) {
    let Observation::Response { content, .. } = disabled else {
        panic!("valid final line should assemble")
    };
    assert_eq!(content.as_deref(), Some("firstlast"));
}

// ADR0007 sensitivity: every mutation below changes one observed fact; expected
// values stay fixed. Each family calls the SAME characterization/assert_parity
// entrypoint as the real transport tests. No production mutation is required.
// IDs P/C/F join the evidence log to this executable mapping. Fixture setup
// unwraps (bind/read/write/parse) and the timeout are fail-fast infrastructure,
// not acceptance assertions; the HTTP-method/size/path/permit assertions ARE
// acceptance assertions and have their own isolated F counterexamples.
fn rejects(id: &str, assertion: impl FnOnce() + std::panic::UnwindSafe) {
    assert!(
        std::panic::catch_unwind(assertion).is_err(),
        "{id}: mutant survived"
    );
    eprintln!("SENSITIVITY {id}: rejected");
}
fn error() -> Error {
    Error {
        display: "original".into(),
        class: ProviderErrorClass::Unknown,
        status: None,
        retryable: false,
    }
}
fn mutate_error(error: &mut Error, field: usize) {
    match field {
        0 => error.display.push_str(" WRONG"),
        1 => error.class = ProviderErrorClass::Auth,
        2 => error.status = Some(401),
        3 => error.retryable = !error.retryable,
        _ => unreachable!(),
    }
}
#[test]
fn parity_oracle_rejects_every_observation_field_mutation() {
    let good = Observation::Error(error());
    assert_parity(&good, &good);
    for field in 0..4 {
        let mut changed = error();
        mutate_error(&mut changed, field);
        rejects(&format!("P-error-{field}"), || {
            assert_parity(&good, &Observation::Error(changed))
        });
    }
    let good = Observation::Response {
        content: Some("text".into()),
        full: "all response fields".into(),
    };
    assert_parity(&good, &good);
    for (id, bad) in [
        (
            "P-response-content",
            Observation::Response {
                content: None,
                full: "all response fields".into(),
            },
        ),
        (
            "P-response-full",
            Observation::Response {
                content: Some("text".into()),
                full: "changed usage/tool/thinking/stop fields".into(),
            },
        ),
        ("P-variant", Observation::Error(error())),
    ] {
        rejects(id, || assert_parity(&good, &bad));
    }
    let good = Observation::Events(vec![
        Event::Error {
            raw: "original".into(),
            classified: error(),
        },
        Event::Other("Done".into()),
    ]);
    assert_parity(&good, &good);
    for field in 0..9 {
        let mut bad = good.clone();
        let Observation::Events(events) = &mut bad else {
            unreachable!()
        };
        match field {
            0..=3 => {
                let Event::Error { classified, .. } = &mut events[0] else {
                    unreachable!()
                };
                mutate_error(classified, field);
            }
            4 => {
                let Event::Error { raw, .. } = &mut events[0] else {
                    unreachable!()
                };
                raw.push_str(" WRONG");
            }
            5 => events[1] = Event::Other("changed Done".into()),
            6 => events.push(Event::Other("duplicate".into())),
            7 => {
                events.remove(0);
            }
            8 => events.swap(0, 1),
            _ => unreachable!(),
        }
        rejects(&format!("P-event-{field}"), || assert_parity(&good, &bad));
    }
}

#[test]
fn fixture_oracles_reject_each_isolated_counterexample() {
    use quecto::domain::inference_admission::Feedback;
    let good = Snapshot {
        grants: 1,
        finishes: vec![Feedback::Failure],
        abandoned: 0,
    };
    oracle::permit(&good);
    for field in 0..3 {
        let mut bad = good.clone();
        match field {
            0 => bad.grants = 0,
            1 => bad.finishes.clear(),
            2 => bad.abandoned = 1,
            _ => unreachable!(),
        }
        rejects(&format!("F0{}", field + 1), || oracle::permit(&bad));
    }
    for (id, limit) in [("F04", 32768), ("F05", 1_048_576)] {
        oracle::request_size(limit - 1, limit);
        rejects(id, || oracle::request_size(limit, limit));
    }
    oracle::post("POST /responses HTTP/1.1");
    rejects("F06", || oracle::post("GET /responses HTTP/1.1"));
    for (id, index) in [("F07", 0), ("F08", 1)] {
        oracle::grants_at_send(index, index);
        rejects(id, || oracle::grants_at_send(1 - index, index));
    }
    oracle::disabled_grants(0);
    rejects("F09", || oracle::disabled_grants(1));
    let good = vec!["POST /responses HTTP/1.1".to_owned(); 2];
    oracle::paths(&good, "/responses");
    rejects("F10", || oracle::paths(&good[..1], "/responses"));
    let mut bad = good.clone();
    bad[0] = "POST /wrong HTTP/1.1".into();
    rejects("F11", || oracle::paths(&bad, "/responses"));
    let mut bad = good.clone();
    bad[1] = "POST /wrong HTTP/1.1".into();
    rejects("F12", || oracle::paths(&bad, "/responses"));
}

// One helper runs a valid characterization first, then changes each field used
// by that characterization in isolation. Removing/retagging observations covers
// the explicit shape/variant guards before their contained field assertions.
fn characterize_sensitivity(
    id: &str,
    good: Observation,
    fields: &[usize],
    check: impl Fn(&Observation) + std::panic::RefUnwindSafe,
) {
    check(&good);
    let wrong_variant = match good {
        Observation::Error(_) => Observation::Events(vec![]),
        _ => Observation::Error(error()),
    };
    rejects(&format!("{id}-shape"), || check(&wrong_variant));
    for field in fields {
        let mut bad = good.clone();
        match &mut bad {
            Observation::Error(error) => mutate_error(error, *field),
            Observation::Response { content, .. } => *content = Some("WRONG".into()),
            Observation::Events(events) => match field {
                4 => {
                    let Event::Error { raw, .. } = &mut events[0] else {
                        unreachable!()
                    };
                    *raw = "WRONG".into();
                }
                5 => events.push(events[0].clone()),
                6 => events[0] = Event::Other("Done".into()),
                _ => {
                    let Event::Error { classified, .. } = &mut events[0] else {
                        unreachable!()
                    };
                    mutate_error(classified, *field);
                }
            },
        }
        // Prefix characterization needs the prefix itself changed, not suffix.
        if *field == 0 {
            if let Observation::Error(error) = &mut bad {
                error.display = "WRONG".into();
            }
        }
        rejects(&format!("{id}-field-{field}"), || check(&bad));
    }
}

#[test]
fn disabled_characterizations_reject_each_expected_fact_counterexample() {
    for leaf in [Leaf::Anthropic, Leaf::Responses, Leaf::OAuth] {
        let raw = if matches!(leaf, Leaf::Anthropic) {
            "HTTP error: error sending request for url (http://127.0.0.1:1/v1/messages)"
        } else {
            "Codex request failed: error sending request for url (http://127.0.0.1:1/responses): connection refused"
        };
        let mut e = error();
        e.display = format!("provider error: {raw}");
        let fields: &[usize] = if matches!(leaf, Leaf::Anthropic) {
            &[0, 1, 3]
        } else {
            &[0, 1]
        };
        characterize_sensitivity(
            &format!("C-refused-{leaf:?}-assembled"),
            Observation::Error(e.clone()),
            fields,
            |out| characterize_refused(out, leaf, "http://127.0.0.1:1"),
        );
        characterize_sensitivity(
            &format!("C-refused-{leaf:?}-incremental"),
            Observation::Events(vec![Event::Error {
                raw: raw.into(),
                classified: e,
            }]),
            &[1, 4, 5, 6],
            |out| characterize_refused(out, leaf, "http://127.0.0.1:1"),
        );
    }
    let mut e = error();
    e.display = "provider error: failed to read response: error decoding response body".into();
    characterize_sensitivity(
        "C-truncated429",
        Observation::Error(e),
        &[0, 1, 2],
        characterize_read_error,
    );
    for leaf in [Leaf::OpenAi, Leaf::Anthropic, Leaf::Responses, Leaf::OAuth] {
        let mut e = error();
        e.class = ProviderErrorClass::RateLimit;
        e.status = Some(429);
        let raw = format!(
            "HTTP 429 from {}: {}... (truncated)",
            leaf.vendor(),
            "x".repeat(4095)
        );
        characterize_sensitivity(
            &format!("C-oversized-{leaf:?}"),
            Observation::Events(vec![Event::Error { raw, classified: e }]),
            &[1, 2, 4, 5, 6],
            |out| characterize_oversized(out, leaf),
        );
    }
    for leaf in [Leaf::Anthropic, Leaf::Responses, Leaf::OAuth] {
        let mut e = error();
        let prefix = if matches!(leaf, Leaf::Anthropic) {
            "failed to read stream"
        } else {
            "failed to read response"
        };
        e.display = format!("provider error: {prefix}: error decoding response body");
        characterize_sensitivity(
            &format!("C-terminal-{leaf:?}"),
            Observation::Error(e),
            &[0, 1],
            |out| characterize_terminal(out, leaf),
        );
    }
    characterize_sensitivity(
        "C-unterminated",
        Observation::Response {
            content: Some("firstlast".into()),
            full: "unused by characterization; parity covers all fields".into(),
        },
        &[0],
        characterize_unterminated,
    );
}
