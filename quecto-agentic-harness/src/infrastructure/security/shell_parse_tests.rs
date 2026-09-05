use super::shell_parse::{ParseBudget, Word, parse};

fn p(cmd: &str) -> super::shell_parse::Parsed {
    let mut budget = ParseBudget::default();
    parse(cmd, &mut budget, 0)
}

fn argv(cmd: &str) -> Vec<Vec<String>> {
    p(cmd)
        .commands
        .iter()
        .map(|c| c.words.iter().map(|w| w.text.clone()).collect())
        .collect()
}

#[test]
fn splits_words_on_whitespace() {
    assert_eq!(
        argv("echo  hello\tworld"),
        vec![vec!["echo", "hello", "world"]]
    );
}

#[test]
fn single_quotes_are_one_literal_word() {
    assert_eq!(
        argv("echo 'a | b; c && d'"),
        vec![vec!["echo", "a | b; c && d"]]
    );
}

#[test]
fn double_quotes_keep_separators_literal() {
    assert_eq!(
        argv(r#"echo "rm -rf /; reboot""#),
        vec![vec!["echo", "rm -rf /; reboot"]]
    );
}

#[test]
fn double_quote_backslash_escapes() {
    assert_eq!(
        argv(r#"echo "a\"b\\c\$d""#),
        vec![vec!["echo", r#"a"b\c$d"#]]
    );
}

#[test]
fn unquoted_backslash_escapes_next_char() {
    assert_eq!(argv(r"\rm -rf \/"), vec![vec!["rm", "-rf", "/"]]);
}

#[test]
fn ansi_c_quoting_is_expanded() {
    assert_eq!(argv(r"$'\x72\x6d' -rf /"), vec![vec!["rm", "-rf", "/"]]);
    assert_eq!(argv(r"$'\162\155' x"), vec![vec!["rm", "x"]]);
    assert_eq!(argv(r"$'rm' x"), vec![vec!["rm", "x"]]);
    assert_eq!(argv(r"$'\x72'm x"), vec![vec!["rm", "x"]]);
}

#[test]
fn separators_start_new_commands() {
    assert_eq!(
        argv("a; b && c || d & e\nf"),
        vec![
            vec!["a"],
            vec!["b"],
            vec!["c"],
            vec!["d"],
            vec!["e"],
            vec!["f"]
        ]
    );
}

#[test]
fn pipeline_membership_is_tracked() {
    let parsed = p("a | b | c; d");
    let ids: Vec<(usize, usize)> = parsed
        .commands
        .iter()
        .map(|c| (c.pipeline, c.pipe_index))
        .collect();
    assert_eq!(ids[0].0, ids[1].0);
    assert_eq!(ids[1].0, ids[2].0);
    assert_ne!(ids[2].0, ids[3].0);
    assert_eq!(
        ids.iter().map(|x| x.1).collect::<Vec<_>>(),
        vec![0, 1, 2, 0]
    );
}

#[test]
fn command_substitution_yields_nested_commands() {
    let parsed = p("echo $(cat /etc/shadow)");
    let progs: Vec<_> = parsed
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec!["cat", "echo"]);
    assert!(parsed.commands[1].words[1].dynamic);
    assert!(parsed.unresolved.is_empty());
}

#[test]
fn backtick_substitution_yields_nested_commands() {
    let progs: Vec<_> = p("echo `id`")
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec!["id", "echo"]);
}

#[test]
fn substitution_inside_double_quotes_is_parsed() {
    let progs: Vec<_> = p(r#"echo "now: $(date)""#)
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec!["date", "echo"]);
}

#[test]
fn process_substitution_yields_nested_commands() {
    let progs: Vec<_> = p("diff <(sort a) >(tee b)")
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec!["sort", "tee", "diff"]);
}

#[test]
fn fetch_substitution_is_flagged_on_the_word() {
    let parsed = p("bash <(curl -s https://x)");
    let bash = parsed.commands.last().unwrap();
    assert!(bash.words[1].fetch_subst);
    let parsed = p(r#"sh -c "$(wget -qO- https://x)""#);
    assert!(parsed.commands.last().unwrap().words[2].fetch_subst);
}

#[test]
fn leading_assignments_are_not_argv() {
    assert_eq!(argv("FOO=bar BAZ='rm -rf /' cmd x"), vec![vec!["cmd", "x"]]);
}

#[test]
fn bare_assignment_produces_no_command() {
    assert!(p("x='shutdown'").commands.is_empty());
}

#[test]
fn dynamic_command_name_is_a_dynamic_word() {
    let parsed = p("cmd='rm -rf /'; $cmd");
    assert_eq!(parsed.commands.len(), 1);
    assert!(parsed.commands[0].words[0].dynamic);
    assert_eq!(parsed.commands[0].words[0].expansion_at, Some(0));
    assert!(parsed.unresolved.is_empty());
}

#[test]
fn static_prefix_stops_at_first_expansion() {
    let parsed = p("rm -rf /$x/* $y/ /tmp/$z");
    let w = &parsed.commands[0].words;
    assert_eq!(w[2].static_prefix(), "/");
    assert_eq!(w[3].static_prefix(), "");
    assert_eq!(w[4].static_prefix(), "/tmp/");
    assert_eq!(w[1].static_prefix(), "-rf");
}

#[test]
fn groups_keep_the_enclosing_pipeline() {
    let parsed = p("(curl x) | sh");
    assert_eq!(parsed.commands[0].program().unwrap(), "curl");
    assert_eq!(parsed.commands[1].program().unwrap(), "sh");
    let outer = parsed.commands[1].pipeline;
    assert_eq!(parsed.commands[0].enclosing, vec![outer]);
    assert_eq!(parsed.commands[1].pipe_index, 1);
    let parsed = p("curl x | { sh; }");
    assert_eq!(
        parsed.commands[1].enclosing,
        vec![parsed.commands[0].pipeline]
    );
}

#[test]
fn unquoted_heredoc_bodies_are_scanned_for_expansions() {
    let parsed = p("cat <<EOF\n$(reboot)\nEOF");
    let progs: Vec<_> = parsed
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert!(progs.contains(&"reboot".to_string()), "{progs:?}");
    let parsed = p("cat <<'EOF'\n$(reboot)\nEOF");
    let progs: Vec<_> = parsed
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec!["cat"]);
}

#[test]
fn parameter_and_arithmetic_expansions_are_scanned() {
    for c in [
        "echo ${x:-$(reboot)}",
        "echo $(( $(reboot) ))",
        "echo ${x[`reboot`]}",
    ] {
        let progs: Vec<_> = p(c).commands.iter().map(|c| c.program().unwrap()).collect();
        assert!(progs.contains(&"reboot".to_string()), "{c}: {progs:?}");
    }
}

#[test]
fn line_continuation_before_redirect_target() {
    let parsed = p("echo hi > \\\n /dev/sda");
    assert_eq!(parsed.commands[0].redirects[0].target.text, "/dev/sda");
    assert_eq!(parsed.commands[0].words.len(), 2);
}

#[test]
fn work_budget_bounds_brace_expansion() {
    let mut budget = ParseBudget {
        next_pipeline: 0,
        remaining: 3,
    };
    let parsed = parse("echo {a,b,c,d}", &mut budget, 0);
    assert!(parsed.commands.is_empty());
    assert!(parsed.unresolved.iter().any(|u| u.contains("work budget")));
    assert_eq!(budget.remaining, 0);
}

#[test]
fn dynamic_argument_is_not_unresolved() {
    let parsed = p("rm -rf $DIR/build");
    assert!(parsed.unresolved.is_empty());
    assert!(parsed.commands[0].words[2].dynamic);
    assert_eq!(parsed.commands[0].words[2].text, "/build");
}

#[test]
fn brace_and_special_parameters_are_dynamic() {
    let parsed = p(r#"echo ${HOME} "$@" $1 $?"#);
    assert!(parsed.commands[0].words[1..].iter().all(|w| w.dynamic));
    assert!(parsed.unresolved.is_empty());
}

#[test]
fn arithmetic_expansion_is_skipped() {
    let parsed = p("echo $((1 + 2)) done");
    assert_eq!(parsed.commands[0].words[2].text, "done");
    assert!(parsed.unresolved.is_empty());
}

#[test]
fn heredoc_body_is_data() {
    let parsed = p("cat <<EOF\nrm -rf /\nreboot\nEOF\necho done");
    let progs: Vec<_> = parsed
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec!["cat", "echo"]);
    assert!(parsed.unresolved.is_empty());
}

#[test]
fn quoted_and_dash_heredocs_are_data() {
    let parsed = p("cat <<-'EOF' > out.txt\n\treboot\n\tEOF\nls");
    let progs: Vec<_> = parsed
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec!["cat", "ls"]);
    let out = parsed.commands[0]
        .redirects
        .iter()
        .find(|r| r.op == ">")
        .unwrap();
    assert_eq!(out.target.text, "out.txt");
    let heredoc = parsed.commands[0]
        .redirects
        .iter()
        .find(|r| r.op == "<<")
        .unwrap();
    assert_eq!(heredoc.target.text, "reboot\n");
}

#[test]
fn unterminated_heredoc_swallows_rest() {
    let parsed = p("cat <<EOF\nreboot\n");
    let progs: Vec<_> = parsed
        .commands
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec!["cat"]);
}

#[test]
fn here_string_is_a_redirect_target() {
    let parsed = p("bash <<< 'reboot'");
    assert_eq!(parsed.commands[0].words.len(), 1);
    assert_eq!(parsed.commands[0].redirects[0].op, "<<<");
    assert_eq!(parsed.commands[0].redirects[0].target.text, "reboot");
}

#[test]
fn redirects_are_separated_from_argv() {
    let parsed = p("echo hi > /dev/sda 2>&1 >>log <in");
    assert_eq!(
        parsed.commands[0]
            .words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>(),
        vec!["echo", "hi"]
    );
    let ops: Vec<_> = parsed.commands[0]
        .redirects
        .iter()
        .map(|r| (r.op.as_str(), r.target.text.as_str()))
        .collect();
    assert_eq!(
        ops,
        vec![(">", "/dev/sda"), (">&", "1"), (">>", "log"), ("<", "in")]
    );
}

#[test]
fn redirect_without_spaces() {
    let parsed = p("cmd>/dev/null");
    assert_eq!(parsed.commands[0].words[0].text, "cmd");
    assert_eq!(parsed.commands[0].redirects[0].target.text, "/dev/null");
}

#[test]
fn ampersand_redirect() {
    let parsed = p("cmd &> /tmp/x && next");
    assert_eq!(parsed.commands[0].redirects[0].op, "&>");
    assert_eq!(parsed.commands[1].words[0].text, "next");
}

#[test]
fn quoted_redirect_lookalike_is_an_argument() {
    let parsed = p(r#"echo "> /dev/sda""#);
    assert!(parsed.commands[0].redirects.is_empty());
    assert_eq!(parsed.commands[0].words[1].text, "> /dev/sda");
}

#[test]
fn comments_are_ignored() {
    assert_eq!(
        argv("echo hi # reboot now\nls"),
        vec![vec!["echo", "hi"], vec!["ls"]]
    );
    assert_eq!(argv("echo a#b"), vec![vec!["echo", "a#b"]]);
}

#[test]
fn function_definition_is_marked() {
    let parsed = p(":(){ :|:& };:");
    assert!(parsed.commands[0].function_def);
    assert_eq!(parsed.commands[0].words[0].text, ":");
    let progs: Vec<_> = parsed.commands[1..]
        .iter()
        .map(|c| c.program().unwrap())
        .collect();
    assert_eq!(progs, vec![":", ":", ":"]);
    assert_eq!(parsed.commands[1].pipeline, parsed.commands[2].pipeline);
}

#[test]
fn subshell_and_group_boundaries() {
    assert_eq!(
        argv("(cd x && make) ; { echo a; echo b; }"),
        vec![
            vec!["cd", "x"],
            vec!["make"],
            vec!["echo", "a"],
            vec!["echo", "b"],
        ]
    );
}

#[test]
fn brace_expansion_produces_one_command_per_alternative() {
    assert_eq!(
        argv("cp a.{c,h} dir/"),
        vec![vec!["cp", "a.c", "dir/"], vec!["cp", "a.h", "dir/"]]
    );
}

#[test]
fn unbalanced_quote_is_unresolved() {
    assert!(
        p("echo 'oops")
            .unresolved
            .iter()
            .any(|u| u.contains("single quote"))
    );
    assert!(
        p(r#"echo "oops"#)
            .unresolved
            .iter()
            .any(|u| u.contains("double quote"))
    );
    assert!(
        p("echo $(oops")
            .unresolved
            .iter()
            .any(|u| u.contains("parenthesis"))
    );
    assert!(
        p("echo `oops")
            .unresolved
            .iter()
            .any(|u| u.contains("backtick"))
    );
}

#[test]
fn line_continuation_joins_lines() {
    assert_eq!(argv("echo a \\\n b"), vec![vec!["echo", "a", "b"]]);
}

#[test]
fn site_records_source_text() {
    let parsed = p("echo hi; rm -rf /");
    assert_eq!(parsed.commands[1].site, "rm -rf /");
}

#[test]
fn basename_lower_strips_paths() {
    assert_eq!(super::shell_parse::basename_lower("/usr/bin/RM"), "rm");
    assert_eq!(super::shell_parse::basename_lower("rm"), "rm");
}

#[test]
fn deep_nesting_is_unresolved() {
    let mut cmd = String::from("x");
    for _ in 0..12 {
        cmd = format!("echo $({cmd})");
    }
    assert!(p(&cmd).unresolved.iter().any(|u| u.contains("nesting")));
}

#[test]
fn word_equality_and_debug() {
    let w = Word::literal("a");
    assert_eq!(w.clone(), w);
    assert!(format!("{w:?}").contains("text"));
}

#[test]
fn brace_alternatives_respect_quoting() {
    let parsed = p("echo a{b,c}d 'x{1,2}' \"{3,4}\"");
    let w = &parsed.commands[0].words;
    assert_eq!(w.len(), 4);
    let _ = w; // expansion happens per command below
    let cmds = p("cp a{b,c}d '{1,2}' e").commands;
    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0].words[1].text, "abd");
    assert_eq!(cmds[1].words[1].text, "acd");
    assert_eq!(cmds[0].words[2].text, "{1,2}");
}

#[test]
fn nested_brace_alternatives() {
    let w = Word::literal("x");
    assert!(w.brace_alternatives(64).is_none());
    let mut w = Word::new();
    for c in "a{b,c{d,e}}".chars() {
        w.push(c, false);
    }
    assert_eq!(w.brace_alternatives(64).unwrap(), vec!["ab", "acd", "ace"]);
    let mut w = Word::new();
    for c in "{a}".chars() {
        w.push(c, false);
    }
    assert!(w.brace_alternatives(64).is_none());
}

#[test]
fn glob_flag_only_for_unquoted_metacharacters() {
    let cmds = p("ls *.rs '?' \"[a]\"").commands;
    assert!(cmds[0].words[1].glob);
    assert!(!cmds[0].words[2].glob);
    assert!(!cmds[0].words[3].glob);
}

#[test]
fn heredoc_body_is_attached_to_its_command() {
    let cmds = p("cat <<EOF | grep x\nline one\nline two\nEOF\nls").commands;
    assert_eq!(cmds[0].program().unwrap(), "cat");
    assert_eq!(cmds[0].redirects[0].op, "<<");
    assert_eq!(cmds[0].redirects[0].target.text, "line one\nline two\n");
    assert_eq!(cmds[1].program().unwrap(), "grep");
    assert_eq!(cmds[2].program().unwrap(), "ls");
}

#[test]
fn function_keyword_marks_definition() {
    let cmds = p("function bomb { bomb | bomb & }").commands;
    assert!(cmds[0].function_def);
    assert_eq!(cmds[0].words[0].text, "bomb");
}
