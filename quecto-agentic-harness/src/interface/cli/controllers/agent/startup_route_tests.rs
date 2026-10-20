use super::*;

fn unknown() -> RouteCheck {
    RouteCheck::UnknownProvider {
        provider: "openai".into(),
        configured: vec!["fireworks".into(), "openai-oauth".into()],
    }
}

#[test]
fn a_spawned_child_with_an_unroutable_model_refuses_to_start_naming_the_providers() {
    let StartupRoute::Refuse(message) = startup_route(unknown(), "openai/gpt-5.2", true) else {
        panic!("a spawned child must refuse");
    };
    assert!(
        message.contains("openai/gpt-5.2") && message.contains("fireworks, openai-oauth"),
        "{message}"
    );
}

#[test]
fn the_root_starts_with_a_warning_so_a_bad_default_never_locks_the_owner_out() {
    assert!(matches!(
        startup_route(unknown(), "openai/gpt-5.2", false),
        StartupRoute::Warn(_)
    ));
}

#[test]
fn a_routable_model_proceeds_either_way() {
    for spawned in [true, false] {
        assert_eq!(
            startup_route(RouteCheck::Routable, "fireworks/glm", spawned),
            StartupRoute::Proceed
        );
    }
}

/// #2435: a configured model its provider does not list starts with a
/// neutral warning naming the remedies, never a crash; retirement is named
/// only for a model #2435 retired (review round 1 L5).
#[test]
fn an_unlisted_startup_model_warns_with_the_remedies() {
    let unlisted = CatalogueStanding::Unlisted {
        provider: "openai-oauth".into(),
    };
    assert_eq!(
        standing_warning(&unlisted, "openai-oauth/gpt-typo").as_deref(),
        Some(
            "agent: warning: provider `openai-oauth` does not list model \
             `openai-oauth/gpt-typo`; it is sent as-is with no known limits. Declare it in \
             models.json, or choose a listed model (list_models, /model)"
        )
    );
    let retired = CatalogueStanding::Retired {
        provider: "openai-oauth".into(),
    };
    assert_eq!(
        standing_warning(&retired, "openai-oauth/gpt-5.5").as_deref(),
        Some(
            "agent: warning: model `openai-oauth/gpt-5.5` was retired from the built-in models \
             of `openai-oauth` (#2435); it is sent as-is with no known limits. Declare it in \
             models.json to keep it, or choose a listed model (list_models, /model)"
        )
    );
}

#[test]
fn a_listed_startup_model_starts_quietly_and_a_refused_one_names_the_reason() {
    assert_eq!(
        standing_warning(&CatalogueStanding::Listed, "openai-oauth/gpt-6.1-sol"),
        None
    );
    assert_eq!(
        standing_warning(
            &CatalogueStanding::UncataloguedProvider,
            "openrouter/some/model"
        ),
        None,
        "an endpoint the catalogue cannot enumerate is no reason to warn"
    );
    assert_eq!(
        standing_warning(
            &CatalogueStanding::RefusedForAccount("no ChatGPT".into()),
            "openai-oauth/mini"
        )
        .as_deref(),
        Some(
            "agent: warning: model `openai-oauth/mini` was refused for this account or auth \
             mode (no ChatGPT); choose a listed model (list_models, /model)"
        )
    );
}
