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
