use super::super::guidance_projection::GuidanceProjection;
use crate::domain::agents::value_objects::turn_outcome::DECLARE_GUIDANCE;
use crate::domain::conversation::value_objects::message::Message;

fn view_and_restore(canonical: &str, bound: bool, enabled: bool) -> String {
    view_with_append(canonical, bound, enabled, None)
}

fn view_with_append(
    canonical: &str,
    bound: bool,
    enabled: bool,
    appended: Option<&super::super::GuidanceAppend>,
) -> String {
    let mut messages = vec![Message::system(canonical)];
    let id = messages[0].id();
    let tokens = messages[0].estimated_tokens();
    let owned = bound.then_some(id);
    let mut appended = appended.cloned();
    if let Some(appended) = &mut appended {
        appended.bind(owned);
    }
    let priced =
        GuidanceProjection::token_change(&messages, owned, Some(enabled), appended.as_ref());
    let view = {
        let mut loan =
            GuidanceProjection::new(&mut messages, owned, Some(enabled), appended.as_ref());
        let projected = &loan.messages()[0];
        assert_eq!(projected.id(), id, "provider projection preserves identity");
        match priced {
            (0, 0) => {}
            (canonical_tokens, projected_tokens) => {
                assert_eq!(canonical_tokens, tokens);
                assert_eq!(projected_tokens, projected.estimated_tokens());
            }
        }
        projected.content.clone()
    };
    assert_eq!(
        messages[0].content, canonical,
        "drop restores canonical text"
    );
    assert_eq!(messages[0].id(), id);
    assert_eq!(
        messages[0].estimated_tokens(),
        tokens,
        "canonical cache restored"
    );
    let cloned = messages[0].clone();
    assert_eq!(cloned.content, canonical, "clone retains canonical view");
    assert_eq!(cloned.estimated_tokens(), tokens);
    view
}

#[test]
fn round3_guidance_disabled_preserves_caller_owned_middle_without_extensions() {
    let canonical = format!("Caller policy before.\n{DECLARE_GUIDANCE}\nCaller policy after.");
    assert_eq!(
        view_and_restore(&canonical, true, false),
        canonical,
        "binding a composed head must not confer ownership of caller text in its middle"
    );
}

#[test]
fn round3_guidance_disabled_preserves_caller_owned_end_without_extensions() {
    let canonical = format!("Caller-authored policy.\n{DECLARE_GUIDANCE}");
    assert_eq!(
        view_and_restore(&canonical, true, false),
        canonical,
        "a matching suffix alone is not proof this code appended guidance"
    );
}

#[test]
fn round3_guidance_control_unbound_matching_text_is_untouched() {
    for canonical in [
        format!("Before.\n{DECLARE_GUIDANCE}\nAfter."),
        format!("Before.\n{DECLARE_GUIDANCE}"),
    ] {
        for enabled in [false, true] {
            assert_eq!(view_and_restore(&canonical, false, enabled), canonical);
        }
    }
}

#[test]
fn round3_guidance_control_framed_owned_removal_preserves_caller_prefix() {
    let prefix = format!("Caller-owned sentence: {DECLARE_GUIDANCE}\nOther caller policy.");
    let canonical = format!(
        "{prefix}\n\n<extensions>\nContent length: {} bytes\n\n{DECLARE_GUIDANCE}\n\n</extensions>",
        DECLARE_GUIDANCE.len()
    );
    let disabled =
        format!("{prefix}\n\n<extensions>\nContent length: 0 bytes\n\n\n\n</extensions>");
    for enabled in [true, false, true, false] {
        let expected = if enabled { &canonical } else { &disabled };
        assert_eq!(view_and_restore(&canonical, true, enabled), *expected);
    }
}

#[test]
fn round3_guidance_control_enabled_once_toggle_restore_and_clone() {
    let plain = "Canonical caller policy without extensions.";
    let enabled = view_and_restore(plain, true, true);
    assert_eq!(enabled.matches(DECLARE_GUIDANCE).count(), 1);
    let owned = vec![Message::system(enabled.clone())];
    let clone = owned.clone();
    assert_eq!(clone[0].id(), owned[0].id());
    for messages in [&owned, &clone] {
        for effective in [true, false, true, false, true] {
            let view = view_and_restore(&messages[0].content, true, effective);
            assert_eq!(
                view.matches(DECLARE_GUIDANCE).count(),
                usize::from(effective)
            );
            assert!(view.starts_with(plain), "caller policy remains visible");
            let (_, framed) = view
                .rsplit_once("\n\n<extensions>\nContent length: ")
                .unwrap();
            let (length, body) = framed.split_once(" bytes\n\n").unwrap();
            assert_eq!(
                length.parse::<usize>().unwrap(),
                body.strip_suffix("\n\n</extensions>").unwrap().len()
            );
        }
    }
}

#[test]
fn round3_guidance_control_actual_unframed_appended_snippets_toggle_restore() {
    use crate::interface::cli::controllers::active_runtime_2708_tests::{compose, isolated};
    let true = isolated(
        concat!(
            module_path!(),
            "::round3_guidance_control_actual_unframed_appended_snippets_toggle_restore"
        ),
        &["1"],
    ) else {
        return;
    };
    // This string was appended by the real registry builder, before framing.
    // Unlike the caller suffix RED above, its provenance is known here.
    let composed = compose(true);
    assert!(
        composed.runtime.is_some(),
        "actual guarded root composition"
    );
    let canonical = &composed.snippets;
    assert_eq!(
        view_and_restore(canonical, true, false),
        *canonical,
        "identical caller bytes without append provenance stay untouched"
    );
    assert!(!canonical.contains("<extensions>"));
    assert_eq!(canonical.matches(DECLARE_GUIDANCE).count(), 1);
    let without_owned = canonical
        .strip_suffix(DECLARE_GUIDANCE)
        .expect("actual composition appends guidance as the last snippet");
    for enabled in [true, false, true, false, true] {
        let expected = if enabled {
            canonical.as_str()
        } else {
            without_owned
        };
        assert_eq!(
            view_with_append(canonical, true, enabled, composed.guidance_append.as_ref()),
            expected
        );
    }
}
