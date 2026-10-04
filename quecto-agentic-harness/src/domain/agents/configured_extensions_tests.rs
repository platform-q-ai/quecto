use super::*;

fn spec(args: &[&str], env: &[(&str, &str)], children: bool) -> ExtensionSpec {
    ExtensionSpec {
        name: "browser-task".into(),
        command: "/usr/bin/browser-task".into(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        env: env
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        children,
    }
}

const VALUES: PlaceholderValues<'static> = PlaceholderValues {
    socket: "/run/a.sock",
    agent_id: "main",
    state_dir: "/base/extensions/browser-task/main",
};

#[test]
fn placeholders_expand_in_args_and_env_values_and_other_braces_stay() {
    let expanded = spec(
        &[
            "--socket",
            "{socket}",
            "--profile={state_dir}/p",
            r#"{"a":1}"#,
            "{ x }",
            "{",
        ],
        &[("AGENT", "{agent_id}"), ("{socket}", "kept")],
        true,
    )
    .expanded(&VALUES)
    .unwrap();
    assert_eq!(
        expanded.args,
        [
            "--socket",
            "/run/a.sock",
            "--profile=/base/extensions/browser-task/main/p",
            r#"{"a":1}"#,
            "{ x }",
            "{"
        ]
    );
    assert_eq!(expanded.env["AGENT"], "main");
    assert_eq!(expanded.env["{socket}"], "kept", "keys are never expanded");
    assert_eq!(expanded.command, "/usr/bin/browser-task");
}

#[test]
fn an_unknown_placeholder_is_an_error_naming_it() {
    for template in ["{sockets}", "a{AGENT_ID}b", "{state_dir}{home}"] {
        let error = check_placeholders(template).unwrap_err();
        assert!(error.contains("unknown placeholder"), "{error}");
        assert!(
            error.contains("{socket}, {agent_id} and {state_dir}"),
            "{error}"
        );
        assert!(spec(&[template], &[], true).expanded(&VALUES).is_err());
    }
    assert!(check_placeholders("{socket}:{agent_id}:{state_dir}").is_ok());
    assert!(
        check_placeholders("{sockets}")
            .unwrap_err()
            .contains("{sockets}")
    );
}

#[test]
fn a_child_launches_only_extensions_marked_for_children() {
    assert!(spec(&[], &[], false).launches_for(AgentRole::TopLevel));
    assert!(spec(&[], &[], true).launches_for(AgentRole::LocalChild));
    assert!(!spec(&[], &[], false).launches_for(AgentRole::LocalChild));
}

#[test]
fn the_agent_id_is_main_at_the_top_and_the_childs_own_id_below() {
    assert_eq!(AgentRole::of(false).agent_id(Some("named")), "main");
    assert_eq!(AgentRole::of(true).agent_id(Some("0f3c")), "0f3c");
    assert_eq!(AgentRole::of(true).agent_id(None), "main");
}

#[test]
fn exit_codes_2_and_3_are_never_restarted() {
    let now = Instant::now();
    let mut budget = RestartBudget::default();
    assert_eq!(
        budget.after_exit(&ExtensionExit::Code(2), now),
        AfterExit::Stop(StopReason::CommandLineError)
    );
    assert_eq!(
        budget.after_exit(&ExtensionExit::Code(3), now),
        AfterExit::Stop(StopReason::ToolsRefused)
    );
}

#[test]
fn other_exits_restart_with_doubling_backoff_until_the_window_budget_is_spent() {
    let start = Instant::now();
    let mut budget = RestartBudget::default();
    let exits = [
        ExtensionExit::Code(1),
        ExtensionExit::Signal(9),
        ExtensionExit::Unobservable,
        ExtensionExit::Code(0),
        ExtensionExit::Code(1),
    ];
    for (index, exit) in exits.iter().enumerate() {
        assert_eq!(
            budget.after_exit(exit, start),
            AfterExit::Restart {
                delay: Duration::from_secs(1 << index),
                restart: index + 1
            }
        );
    }
    assert_eq!(
        budget.after_exit(&ExtensionExit::Code(1), start),
        AfterExit::Stop(StopReason::RestartLimit)
    );
    // Once the window has passed, the budget is whole again.
    assert_eq!(
        budget.after_exit(&ExtensionExit::Code(1), start + RESTART_WINDOW),
        AfterExit::Restart {
            delay: Duration::from_secs(1),
            restart: 1
        }
    );
}

#[test]
fn only_trouble_is_a_warning_and_only_starting_or_restarting_is_unsettled() {
    let log = "/base/extensions/x/main/extension.log";
    assert_eq!(ExtensionState::Starting.warning("x", log), None);
    assert_eq!(ExtensionState::Running.warning("x", log), None);
    let restarting = ExtensionState::Restarting {
        exit: ExtensionExit::Code(1),
        restart: 2,
    };
    let warning = restarting.warning("x", log).unwrap();
    assert!(
        warning.contains("exit code 1") && warning.contains("restart 2 of 5"),
        "{warning}"
    );
    assert!(warning.contains(log), "{warning}");
    let unregistered = ExtensionState::Unregistered.warning("x", log).unwrap();
    assert!(unregistered.contains("within 30 s"), "{unregistered}");
    for (reason, says) in [
        (StopReason::CommandLineError, "command-line error"),
        (StopReason::ToolsRefused, "tools were refused"),
        (StopReason::RestartLimit, "not restarted again"),
        (
            StopReason::LaunchFailed("no such file".into()),
            "no such file",
        ),
    ] {
        let stopped = ExtensionState::Stopped { exit: None, reason };
        assert!(stopped.settled());
        let warning = stopped.warning("x", log).unwrap();
        assert!(warning.contains(says), "{warning}");
    }
    assert!(!ExtensionState::Starting.settled());
    assert!(!restarting.settled());
    assert!(ExtensionState::Running.settled());
    assert!(ExtensionState::Unregistered.settled());
}

#[test]
fn an_agent_selects_what_its_role_launches_and_nothing_when_switched_off() {
    let configured = vec![spec(&[], &[], true), {
        let mut parent_only = spec(&[], &[], false);
        parent_only.name = "parent-only".into();
        parent_only
    }];
    let top = AgentExtensions::select(configured.clone(), AgentRole::TopLevel, Some("s"), true);
    assert_eq!((top.agent_id.as_str(), top.specs.len()), ("main", 2));
    let child =
        AgentExtensions::select(configured.clone(), AgentRole::LocalChild, Some("c1"), true);
    assert_eq!(child.agent_id, "c1");
    assert_eq!(child.specs, [spec(&[], &[], true)]);
    let off = AgentExtensions::select(configured, AgentRole::TopLevel, None, false);
    assert!(off.specs.is_empty());
}
