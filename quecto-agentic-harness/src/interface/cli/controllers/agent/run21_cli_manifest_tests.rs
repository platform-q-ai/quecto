use super::*;

#[test]
fn run21_real_resumed_cli_manifest_is_not_guidance_owner() {
    use crate::interface::cli::controllers::active_runtime_2708_tests::{compose, isolated};
    if !isolated(
        concat!(
            module_path!(),
            "::run21_real_resumed_cli_manifest_is_not_guidance_owner"
        ),
        &["unset", "1"],
    ) {
        return;
    }
    let run = run_composition_seeded(
        vec![
            Reply::Call(
                "declare_turn_outcome",
                json!({"outcome":"done","reason":"Complete"}),
            ),
            Reply::Text("Answer"),
        ],
        (false, vec![], None),
        (None, false),
        Some(compose(false)),
        Some("Earlier answer"),
        true,
    );
    assert!(
        !run.manifests.is_empty(),
        "real CLI loaded durable manifest"
    );
    assert!(
        run.manifests
            .iter()
            .all(|m| m == "[context manifest] retained summary"),
        "resumed CLI manifest must remain byte-identical, never receive guidance: {:?}",
        run.manifests
    );
}
