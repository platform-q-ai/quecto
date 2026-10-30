use super::*;

fn assert_owned_view(message: &Message, appended: &GuidanceAppend, expected: &str) {
    let mut messages = vec![message.clone()];
    let tokens = messages[0].estimated_tokens();
    let priced = GuidanceProjection::token_change(
        &messages,
        Some(message.id()),
        Some(false),
        Some(appended),
    );
    {
        let mut loan = GuidanceProjection::new(
            &mut messages,
            Some(message.id()),
            Some(false),
            Some(appended),
        );
        assert_eq!(loan.messages()[0].content, expected);
        let (canonical, projected) = priced;
        if canonical > 0 {
            assert_eq!(canonical, tokens);
            assert_eq!(projected, loan.messages()[0].estimated_tokens());
        }
    }
    assert_eq!(messages[0].content, message.content);
    assert_eq!(messages[0].id(), message.id());
    assert_eq!(messages[0].estimated_tokens(), tokens);
}

#[test]
fn run43_guidance_append_clone_and_stale_identity_are_safe() {
    let mut content = "Caller π".to_string();
    let mut appended = GuidanceAppend::append_to(&mut content);
    let message = Message::system(content.clone());
    appended.bind(Some(message.id()));
    let clone = appended.clone();
    assert_owned_view(&message.clone(), &clone, "Caller π\n\n");
    let caller = Message::system(content);
    assert_owned_view(&caller, &clone, &caller.content);
    appended.bind(Some(caller.id()));
    assert_owned_view(&caller, &appended, &caller.content);
    appended.bind(Some(message.id()));
    assert_owned_view(&message, &appended, &message.content);
    let mut changed = message.clone();
    changed.content.insert_str(0, "New caller text. ");
    changed.invalidate_token_cache();
    assert_owned_view(&changed, &clone, &changed.content);
}

#[test]
fn run43_guidance_append_invalid_ranges_preserve_caller() {
    let mut content = "π".to_string();
    let mut appended = GuidanceAppend::append_to(&mut content);
    let message = Message::system(content);
    appended.bind(Some(message.id()));
    let reversed = std::ops::Range { start: 3, end: 2 };
    for range in [1..2, 0..usize::MAX, reversed, 0..2] {
        let mut invalid = appended.clone();
        invalid.range = range;
        assert_owned_view(&message, &invalid, &message.content);
    }
}
