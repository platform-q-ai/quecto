use quecto::application::agent_turn::use_cases::web_fetch::FetchWebContent;
fn assert_port<T: FetchWebContent + ?Sized>() {}
#[test]
fn fetch_web_content_is_object_safe() {
    assert_port::<dyn FetchWebContent>();
}
