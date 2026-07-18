use super::*;

#[test]
fn inject_system_prompt_inserts_when_history_has_no_system_message() {
    let mut messages = vec![Message::user("hi")];

    inject_system_prompt(&mut messages, "be helpful");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, Role::System);
    assert_eq!(messages[0].content, "be helpful");
    assert_eq!(messages[1].content, "hi");
}

#[test]
fn inject_system_prompt_preserves_existing_real_system_message() {
    let mut messages = vec![Message::system("existing"), Message::user("hi")];

    inject_system_prompt(&mut messages, "new prompt");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "existing");
}

#[test]
fn inject_system_prompt_ignores_empty_prompt() {
    let mut messages = vec![Message::user("hi")];

    inject_system_prompt(&mut messages, "");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, Role::User);
}

#[test]
fn remove_injected_system_prompt_removes_exact_or_prefixed_prompt() {
    let mut messages = vec![Message::system("be helpful\nextra"), Message::user("hi")];

    remove_injected_system_prompt(&mut messages, "be helpful");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, Role::User);
}

#[test]
fn remove_injected_system_prompt_preserves_nonmatching_system_prompt() {
    let mut messages = vec![Message::system("existing"), Message::user("hi")];

    remove_injected_system_prompt(&mut messages, "be helpful");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "existing");
}
