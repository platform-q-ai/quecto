use super::*;

// -- shell_split_repl --

#[test]
fn test_shell_split_repl_basic() {
    let tokens = shell_split_repl("hello world");
    assert_eq!(tokens, vec!["hello", "world"]);
}

#[test]
fn test_shell_split_repl_quotes() {
    let tokens = shell_split_repl("--cron '0 9 * * *' --message Hello");
    assert_eq!(tokens, vec!["--cron", "0 9 * * *", "--message", "Hello"]);
}

#[test]
fn test_shell_split_repl_utf8() {
    let tokens = shell_split_repl("--message 'café résumé'");
    assert_eq!(tokens, vec!["--message", "café résumé"]);

    let tokens = shell_split_repl("hello 世界");
    assert_eq!(tokens, vec!["hello", "世界"]);

    let tokens = shell_split_repl("--system '你好世界' task");
    assert_eq!(tokens, vec!["--system", "你好世界", "task"]);
}

#[test]
fn test_shell_split_repl_double_quotes() {
    let tokens = shell_split_repl(r#"--system "You are a translator" task"#);
    assert_eq!(tokens, vec!["--system", "You are a translator", "task"]);
}

#[test]
fn test_shell_split_repl_mixed_quotes() {
    let tokens = shell_split_repl(r#"'single quoted' "double quoted" bare"#);
    assert_eq!(tokens, vec!["single quoted", "double quoted", "bare"]);
}

#[test]
fn test_shell_split_repl_empty_string() {
    assert!(shell_split_repl("").is_empty());
}

#[test]
fn test_shell_split_repl_only_spaces() {
    assert!(shell_split_repl("     ").is_empty());
}

#[test]
fn test_shell_split_repl_empty_quotes() {
    assert!(shell_split_repl("'' \"\"").is_empty());
}

#[test]
fn test_shell_split_repl_adjacent_tokens() {
    let tokens = shell_split_repl("a'b'c");
    assert_eq!(tokens, vec!["a", "b", "c"]);
}

#[test]
fn test_shell_split_repl_single_token() {
    assert_eq!(shell_split_repl("hello"), vec!["hello"]);
}

#[test]
fn test_shell_split_repl_preserves_inner_whitespace() {
    assert_eq!(shell_split_repl("'hello   world'"), vec!["hello   world"]);
}

#[test]
fn test_shell_split_repl_unclosed_quote() {
    assert_eq!(shell_split_repl("'unclosed quote"), vec!["unclosed quote"]);
}

// -- parse_agent_args --

#[test]
fn test_agent_with_system() {
    let p = parse_agent_args("researcher --system You are a specialist").unwrap();
    assert_eq!(p.name, "researcher");
    assert_eq!(p.system.as_deref(), Some("You are a specialist"));
    assert!(p.model.is_none());
}

#[test]
fn test_agent_with_system_and_model() {
    let p = parse_agent_args("fast-bot --system Quick answers --model gpt-5-mini").unwrap();
    assert_eq!(p.system.as_deref(), Some("Quick answers"));
    assert_eq!(p.model.as_deref(), Some("gpt-5-mini"));
}

#[test]
fn test_agent_model_only() {
    let p = parse_agent_args("bot --model gpt-5-mini").unwrap();
    assert!(p.system.is_none());
    assert_eq!(p.model.as_deref(), Some("gpt-5-mini"));
}

#[test]
fn test_agent_empty() {
    assert!(
        parse_agent_args("")
            .unwrap_err()
            .contains("missing agent name")
    );
}

#[test]
fn test_agent_name_only() {
    let p = parse_agent_args("nameless").unwrap();
    assert_eq!(p.name, "nameless");
    assert!(p.system.is_none());
    assert!(p.model.is_none());
}

#[test]
fn test_agent_system_requires_value() {
    assert!(
        parse_agent_args("bot --system")
            .unwrap_err()
            .contains("--system requires a value")
    );
}

#[test]
fn test_agent_model_requires_value() {
    assert!(
        parse_agent_args("bot --model")
            .unwrap_err()
            .contains("--model requires a value")
    );
}

#[test]
fn test_agent_system_stops_at_model() {
    let p = parse_agent_args("bot --system Be concise --model gpt-5-mini").unwrap();
    assert_eq!(p.system.as_deref(), Some("Be concise"));
    assert_eq!(p.model.as_deref(), Some("gpt-5-mini"));
}

#[test]
fn test_agent_system_quoted() {
    let p = parse_agent_args("bot --system 'You are a helpful assistant'").unwrap();
    assert_eq!(p.system.as_deref(), Some("You are a helpful assistant"));
}

#[test]
fn test_agent_unknown_flags_skipped() {
    let p = parse_agent_args("bot --unknown value --system test").unwrap();
    assert_eq!(p.system.as_deref(), Some("test"));
}

#[test]
fn test_agent_debug_impl() {
    let p = parse_agent_args("bot --system test").unwrap();
    assert!(format!("{:?}", p).contains("ParsedAgentArgs"));
}

// -- is_valid_agent_name --

#[test]
fn test_valid_names() {
    assert!(is_valid_agent_name("researcher"));
    assert!(is_valid_agent_name("fast-bot"));
    assert!(is_valid_agent_name("my_agent_1"));
    assert!(is_valid_agent_name("A"));
    assert!(is_valid_agent_name("aZ-_09"));
    assert!(is_valid_agent_name(&"a".repeat(64)));
}

#[test]
fn test_invalid_names() {
    assert!(!is_valid_agent_name(""));
    assert!(!is_valid_agent_name("../escape"));
    assert!(!is_valid_agent_name("bad name"));
    assert!(!is_valid_agent_name("bad/name"));
    assert!(!is_valid_agent_name("bad.name"));
    assert!(!is_valid_agent_name(&"a".repeat(65)));
    assert!(!is_valid_agent_name("café"));
    assert!(!is_valid_agent_name("日本語"));
}

// -- parse_spawn_args --

#[test]
fn test_spawn_simple_task() {
    let p = parse_spawn_args("What is the meaning of life?").unwrap();
    assert_eq!(p.task, "What is the meaning of life?");
    assert!(p.agent.is_none());
    assert!(!p.help);
}

#[test]
fn test_spawn_with_agent() {
    let p = parse_spawn_args("--agent researcher What is new?").unwrap();
    assert_eq!(p.agent.as_deref(), Some("researcher"));
    assert_eq!(p.task, "What is new?");
}

#[test]
fn test_spawn_with_system() {
    let p = parse_spawn_args("--system 'You are a translator' Translate: hello").unwrap();
    assert_eq!(p.system.as_deref(), Some("You are a translator"));
    assert_eq!(p.task, "Translate: hello");
}

#[test]
fn test_spawn_with_model() {
    let p = parse_spawn_args("--model gpt-5-mini Summarize briefly").unwrap();
    assert_eq!(p.model.as_deref(), Some("gpt-5-mini"));
}

#[test]
fn test_spawn_with_max_time() {
    let p = parse_spawn_args("--max-time 30 Slow task").unwrap();
    assert_eq!(p.max_time, Some(30));
}

#[test]
fn test_spawn_help() {
    assert!(parse_spawn_args("--help").unwrap().help);
}

#[test]
fn test_spawn_empty() {
    assert!(parse_spawn_args("").unwrap().task.is_empty());
}

#[test]
fn test_spawn_combined_flags() {
    let p = parse_spawn_args("--agent bot --system 'Custom prompt' --max-time 60 Do the thing")
        .unwrap();
    assert_eq!(p.agent.as_deref(), Some("bot"));
    assert_eq!(p.system.as_deref(), Some("Custom prompt"));
    assert_eq!(p.max_time, Some(60));
    assert_eq!(p.task, "Do the thing");
}

#[test]
fn test_spawn_agent_requires_value() {
    assert!(
        parse_spawn_args("--agent")
            .unwrap_err()
            .contains("--agent requires a value")
    );
}

#[test]
fn test_spawn_system_requires_value() {
    assert!(
        parse_spawn_args("--system")
            .unwrap_err()
            .contains("--system requires a value")
    );
}

#[test]
fn test_spawn_model_requires_value() {
    assert!(
        parse_spawn_args("--model")
            .unwrap_err()
            .contains("--model requires a value")
    );
}

#[test]
fn test_spawn_max_time_requires_value() {
    assert!(
        parse_spawn_args("--max-time")
            .unwrap_err()
            .contains("--max-time requires a value")
    );
}

#[test]
fn test_spawn_max_time_invalid() {
    assert!(
        parse_spawn_args("--max-time abc task")
            .unwrap_err()
            .contains("invalid --max-time")
    );
}

#[test]
fn test_spawn_help_with_task() {
    let p = parse_spawn_args("--help Do something").unwrap();
    assert!(p.help);
    assert_eq!(p.task, "Do something");
}

#[test]
fn test_spawn_all_flags() {
    let p = parse_spawn_args(
        "--agent bot --system 'Custom' --model gpt-5 --max-time 120 --help My task",
    )
    .unwrap();
    assert_eq!(p.agent.as_deref(), Some("bot"));
    assert_eq!(p.system.as_deref(), Some("Custom"));
    assert_eq!(p.model.as_deref(), Some("gpt-5"));
    assert_eq!(p.max_time, Some(120));
    assert!(p.help);
    assert_eq!(p.task, "My task");
}

#[test]
fn test_spawn_debug_impl() {
    let p = parse_spawn_args("my task").unwrap();
    assert!(format!("{:?}", p).contains("ParsedSpawnArgs"));
}
