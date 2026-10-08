//! #2435: a model the provider refused for the account in use is never
//! switched to, and the plan says where a model stands in the catalogue.

use super::*;
use crate::application::catalogue::dto::CatalogueStanding;

const HELD: std::time::Duration = std::time::Duration::from_secs(3600);

const REASON: &str = "The 'mini' model is not supported when using Codex with a ChatGPT account.";

fn refused_rig() -> Rig {
    let entries = vec![
        entry("openai-oauth", "mini", None),
        entry("openai-oauth", "sol", None),
    ];
    let rig = rig(entries.clone(), FakeRuntime::over(entries, 1));
    rig.store.record_refusal(
        &ModelRef::parse_qualified("openai-oauth/mini").unwrap(),
        REASON,
        HELD,
    );
    rig
}

#[test]
fn a_switch_to_a_refused_model_is_refused_with_the_reason_and_changes_nothing() {
    let rig = refused_rig();
    let mut runtime = FakeLoop {
        model: "openai-oauth/sol".into(),
        effort: Some(EffortLevel::High),
        ..Default::default()
    };
    let error = rig
        .use_case
        .execute(&mut runtime, "openai-oauth/mini")
        .unwrap_err();
    assert_eq!(
        error,
        ModelSwitchError::RefusedForAccount {
            model: "openai-oauth/mini".into(),
            reason: REASON.into(),
        }
    );
    assert_eq!(
        error.to_string(),
        format!(
            "cannot switch to `openai-oauth/mini`: the provider refused it for this account or \
             auth mode ({REASON}); it is held unavailable for now. Choose another model from \
             list_models"
        )
    );
    assert_eq!(
        runtime.model, "openai-oauth/sol",
        "the session keeps its model"
    );
    assert_eq!(runtime.effort, Some(EffortLevel::High));
}

#[test]
fn a_refused_model_is_never_recorded_as_a_default() {
    let entries = vec![entry("openai-oauth", "mini", None)];
    let persistence = Arc::new(RecordedDefaults::default());
    let rig = rig_persisting(
        entries.clone(),
        FakeRuntime::over(entries, 1),
        persistence.clone(),
    );
    rig.store.record_refusal(
        &ModelRef::parse_qualified("openai-oauth/mini").unwrap(),
        REASON,
        HELD,
    );
    let mut runtime = FakeLoop::default();
    let error = rig
        .use_case
        .execute_with_default(
            &mut runtime,
            "openai-oauth/mini",
            Some(DefaultScope::Global),
        )
        .unwrap_err();
    assert!(matches!(error, ModelSwitchError::RefusedForAccount { .. }));
    assert!(
        persistence.records.lock().unwrap().is_empty(),
        "nothing recorded"
    );
}

#[test]
fn a_bare_id_the_router_sends_to_the_refusing_provider_is_refused_too() {
    let rig = refused_rig();
    let mut runtime = FakeLoop::default();
    assert!(matches!(
        rig.use_case.execute(&mut runtime, "mini"),
        Err(ModelSwitchError::RefusedForAccount { .. })
    ));
    assert_eq!(runtime.model, "");
}

#[test]
fn the_plan_says_where_the_model_stands() {
    let rig = refused_rig();
    let standing = |model: &str| rig.use_case.plan(model).standing;
    assert_eq!(standing("openai-oauth/sol"), CatalogueStanding::Listed);
    assert_eq!(
        standing("openai-oauth/mini"),
        CatalogueStanding::RefusedForAccount(REASON.into())
    );
    assert_eq!(
        standing("openai-oauth/gpt-5.5"),
        CatalogueStanding::Retired {
            provider: "openai-oauth".into()
        },
        "a built-in #2435 retired"
    );
    assert_eq!(
        standing("openai-oauth/gpt-typo"),
        CatalogueStanding::Unlisted {
            provider: "openai-oauth".into()
        },
        "any other id the provider does not list"
    );
    assert_eq!(
        standing("elsewhere/x"),
        CatalogueStanding::UncataloguedProvider,
        "a provider the router does not reach lists nothing"
    );
}

#[test]
fn an_unlisted_model_still_switches() {
    let rig = refused_rig();
    let mut runtime = FakeLoop::default();
    let switched = rig
        .use_case
        .execute(&mut runtime, "openai-oauth/gpt-5.5")
        .unwrap();
    assert!(matches!(
        switched.plan.standing,
        CatalogueStanding::Retired { .. }
    ));
    assert_eq!(runtime.model, "openai-oauth/gpt-5.5");
}

/// A provider the catalogue lists no models for (an open-router endpoint)
/// says nothing of the ids it serves.
#[test]
fn an_id_on_a_provider_that_lists_no_models_is_uncatalogued() {
    let entries = vec![entry("openai-oauth", "sol", None)];
    let runtime = Arc::new(FakeRuntime(Some(Arc::new(
        crate::application::provider_runtime::CatalogueRuntimeSnapshot {
            catalogue: Arc::new(
                crate::domain::catalogue::value_objects::catalogue::resolve_catalogue(
                    1,
                    vec![(SourceLayer::BuiltIn, entries.clone())],
                )
                .snapshot,
            ),
            provider: Arc::new(OrderedRouter(vec![
                "openai-oauth".into(),
                "openrouter".into(),
            ])),
            admission_binding_diagnostic: Default::default(),
        },
    ))));
    let rig = rig(entries, runtime);
    assert_eq!(
        rig.use_case.plan("openrouter/some/model").standing,
        CatalogueStanding::UncataloguedProvider
    );
    assert_eq!(
        rig.use_case.plan("gpt-typo").standing,
        CatalogueStanding::Unlisted {
            provider: "openai-oauth".into()
        },
        "a bare id reaches the first provider, which lists other models"
    );
}

/// Review round 1 L7: a refusal recorded for a model the catalogue does not
/// list (one an account was refused before it was ever listed, or a retired
/// one) still makes the switch refuse, as the refusal's guidance says.
#[test]
fn a_refused_model_the_catalogue_does_not_list_is_refused_too() {
    let rig = refused_rig();
    rig.store.record_refusal(
        &ModelRef::parse_qualified("openai-oauth/gpt-5.5").unwrap(),
        REASON,
        HELD,
    );
    assert_eq!(
        rig.use_case.plan("openai-oauth/gpt-5.5").standing,
        CatalogueStanding::RefusedForAccount(REASON.into())
    );
    let mut runtime = FakeLoop::default();
    assert!(matches!(
        rig.use_case.execute(&mut runtime, "openai-oauth/gpt-5.5"),
        Err(ModelSwitchError::RefusedForAccount { .. })
    ));
}
