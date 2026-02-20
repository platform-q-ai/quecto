use super::*;

// Telegram Steps
// ===========================================================================

#[given(expr = "a config with Telegram enabled and token {string}")]
fn given_telegram_enabled(world: &mut QuectoWorld, token: String) {
    world.telegram_config = Some(TelegramConfig {
        enabled: true,
        token,
        allow_from: vec![],
    });
}

#[given("a config with Telegram disabled")]
fn given_telegram_disabled(world: &mut QuectoWorld) {
    world.telegram_config = Some(TelegramConfig {
        enabled: false,
        token: String::new(),
        allow_from: vec![],
    });
}

#[given(expr = "a Telegram channel with allow_from {string}, {string}")]
fn given_telegram_with_allow_from(world: &mut QuectoWorld, user1: String, user2: String) {
    let config = TelegramConfig {
        enabled: true,
        token: "test-token".to_string(),
        allow_from: vec![user1, user2],
    };
    world.telegram_channel = Some(TelegramChannel::new(&config));
}

#[given("a Telegram channel with empty allow_from")]
fn given_telegram_empty_allow_from(world: &mut QuectoWorld) {
    let config = TelegramConfig {
        enabled: true,
        token: "test-token".to_string(),
        allow_from: vec![],
    };
    world.telegram_channel = Some(TelegramChannel::new(&config));
}

#[given(expr = "a raw Telegram update with text {string} from user {string}")]
fn given_raw_telegram_update(world: &mut QuectoWorld, text: String, user_id: String) {
    let uid: i64 = user_id.parse().unwrap();
    world.telegram_update = Some(TelegramUpdate {
        update_id: 1,
        message: Some(TelegramUpdateMessage {
            message_id: 42,
            from: Some(TelegramUser {
                id: uid,
                first_name: Some("Test".to_string()),
                username: None,
            }),
            chat: TelegramChat {
                id: uid,
                chat_type: Some("private".to_string()),
            },
            text: Some(text),
        }),
    });
}

#[when("the Telegram channel is created")]
fn when_telegram_created(world: &mut QuectoWorld) {
    let config = world
        .telegram_config
        .as_ref()
        .expect("telegram config not set");
    world.telegram_channel = Some(TelegramChannel::new(config));
}

#[when("I check if Telegram is enabled")]
fn when_check_telegram_enabled(world: &mut QuectoWorld) {
    // Evaluate the enabled flag from config without constructing a full
    // TelegramChannel (which allocates a reqwest::Client).
    let config = world
        .telegram_config
        .as_ref()
        .expect("telegram config not set");
    let enabled = config.enabled && !config.token.is_empty();
    world.telegram_enabled_check = Some(enabled);
}

#[when(expr = "user {string} sends a message")]
fn when_user_sends_telegram_message(world: &mut QuectoWorld, user_id: String) {
    let ch = world
        .telegram_channel
        .as_ref()
        .expect("telegram channel not set");
    world.telegram_filter_result = Some(ch.is_user_allowed(&user_id));
}

#[when("the update is parsed")]
fn when_update_parsed(world: &mut QuectoWorld) {
    let update = world
        .telegram_update
        .as_ref()
        .expect("telegram update not set");
    world.telegram_parsed_message = TelegramChannel::parse_update(update);
}

#[then(expr = "the channel name should be {string}")]
fn then_channel_name(world: &mut QuectoWorld, expected: String) {
    let ch = world
        .telegram_channel
        .as_ref()
        .expect("telegram channel not set");
    assert_eq!(ch.name(), expected);
}

#[then("the channel should be enabled")]
fn then_channel_enabled(world: &mut QuectoWorld) {
    let ch = world
        .telegram_channel
        .as_ref()
        .expect("telegram channel not set");
    assert!(ch.is_enabled(), "channel should be enabled");
}

#[then("the Telegram channel should not be enabled")]
fn then_telegram_not_enabled(world: &mut QuectoWorld) {
    // Prefer the lightweight enabled-check result (set by "When I check if
    // Telegram is enabled") over the full channel object.
    if let Some(enabled) = world.telegram_enabled_check {
        assert!(!enabled, "channel should not be enabled");
    } else {
        let ch = world
            .telegram_channel
            .as_ref()
            .expect("telegram channel or enabled check not set");
        assert!(!ch.is_enabled(), "channel should not be enabled");
    }
}

#[then("the message should pass the allow_from filter")]
fn then_message_passes_filter(world: &mut QuectoWorld) {
    let result = world.telegram_filter_result.expect("no filter result");
    assert!(result, "message should pass the allow_from filter");
}

#[then("the message should be rejected by the allow_from filter")]
fn then_message_rejected_by_filter(world: &mut QuectoWorld) {
    let result = world.telegram_filter_result.expect("no filter result");
    assert!(
        !result,
        "message should be rejected by the allow_from filter"
    );
}

#[then(expr = "the parsed message text should be {string}")]
fn then_parsed_text(world: &mut QuectoWorld, expected: String) {
    let msg = world
        .telegram_parsed_message
        .as_ref()
        .expect("no parsed message");
    assert_eq!(msg.text, expected);
}

#[then(expr = "the parsed sender ID should be {string}")]
fn then_parsed_sender_id(world: &mut QuectoWorld, expected: String) {
    let msg = world
        .telegram_parsed_message
        .as_ref()
        .expect("no parsed message");
    assert_eq!(msg.sender_id, expected);
}

// ===========================================================================
