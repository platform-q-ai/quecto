//! #2421: what the change-active-model use case reads a model takes of a
//! conversation's images, and which entry a model id reads: the one the
//! router sends the request to.

use super::*;
use crate::domain::conversation::image_input::ImageInput;

/// #2421: what a model takes of a conversation's images, from its entry:
/// none unless it declares `image`; every image over the Anthropic wire,
/// still images only over the OpenAI one.
fn seeing(provider: &str, model: &str, transport: TransportKind) -> CatalogueEntry {
    let mut seeing = entry(provider, model, Some((50, 1234)));
    seeing.model.capabilities.input_modalities = vec!["text".into(), "image".into()];
    seeing.provider.transport = transport;
    seeing
}

#[test]
fn a_model_takes_images_only_when_its_entry_declares_image_input() {
    let rig = rig(
        vec![
            seeing("acme", "seeing", TransportKind::OpenAiCompletions),
            seeing("claude", "opus", TransportKind::AnthropicMessages),
            entry("acme", "plain", None),
        ],
        FakeRuntime::none(),
    );
    let input = |model: &str| rig.use_case.plan(model).limits.image_input;
    assert_eq!(input("acme/seeing"), ImageInput::StillImages);
    assert_eq!(input("claude/opus"), ImageInput::AllImages);
    assert_eq!(input("acme/plain"), ImageInput::NoImages);
    assert_eq!(input("acme/unknown"), ImageInput::NoImages);
    assert_eq!(input("unknown"), ImageInput::NoImages);
}

/// #2421 review L2: the provider is matched whatever its case, as the
/// router and `qualified` match it.
#[test]
fn a_provider_named_in_another_case_reads_the_same_entry() {
    let rig = rig(
        vec![seeing(
            "openai-oauth",
            "sol",
            TransportKind::OpenAiCompletions,
        )],
        FakeRuntime::none(),
    );
    let limits = rig.use_case.plan("OpenAI-OAuth/sol").limits;
    assert_eq!(limits.image_input, ImageInput::StillImages);
    assert_eq!(limits.max_output_tokens, Some(50));
}

fn routed_over(entries: Vec<CatalogueEntry>, order: Vec<&'static str>) -> Arc<FakeRuntime> {
    let catalogue =
        crate::domain::catalogue::resolve_catalogue(1, vec![(SourceLayer::BuiltIn, entries)])
            .snapshot;
    Arc::new(FakeRuntime(Some(Arc::new(
        crate::application::provider_runtime::CatalogueRuntimeSnapshot {
            catalogue: Arc::new(catalogue),
            provider: Arc::new(OrderedRouter(
                order.into_iter().map(str::to_string).collect(),
            )),
            admission_binding_diagnostic: Default::default(),
        },
    ))))
}

/// #2421 round 2 L1: a bare id reads the entry of the provider the router
/// sends it to, its first, whether or not that provider lists it; one that
/// does not gives no entry, so no image goes (fail closed). This is where
/// the two used to differ: the catalogue read the first provider listing
/// the id while the request went to the router's first.
#[test]
fn a_bare_id_reads_the_routers_first_provider_only() {
    let entries = vec![
        entry("first", "plain", None),
        seeing("second", "sol", TransportKind::OpenAiCompletions),
    ];
    let first_first = rig(
        entries.clone(),
        routed_over(entries.clone(), vec!["first", "second"]),
    );
    let plan = |model: &str| first_first.use_case.plan(model).limits;
    assert_eq!(plan("sol"), ModelLimits::default());
    assert_eq!(plan("second/sol").image_input, ImageInput::StillImages);
    let second_first = rig(
        entries.clone(),
        routed_over(entries, vec!["second", "first"]),
    );
    let limits = second_first.use_case.plan("sol").limits;
    assert_eq!(limits.image_input, ImageInput::StillImages);
    assert_eq!(limits.max_output_tokens, Some(50));
}

/// Before a runtime is composed, the providers are the catalogue's, in its
/// order: a bare id reads the first of them.
#[test]
fn a_bare_id_without_a_runtime_reads_the_first_catalogue_provider() {
    let rig = rig(
        vec![
            seeing("only", "lonely", TransportKind::AnthropicMessages),
            seeing("acme", "plain", TransportKind::OpenAiCompletions),
        ],
        FakeRuntime::none(),
    );
    let input = |model: &str| rig.use_case.plan(model).limits.image_input;
    assert_eq!(input("lonely"), ImageInput::AllImages);
    assert_eq!(input("plain"), ImageInput::NoImages, "not on the first");
}
