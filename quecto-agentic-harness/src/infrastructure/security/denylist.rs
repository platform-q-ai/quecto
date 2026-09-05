// Dangerous-command denylist evaluated over parsed shell structure (#1620).
//
// Rules match on the *execution site* — the program word of a simple command
// and its arguments/redirects — rather than on the raw command string. Prose,
// filenames, source snippets and heredoc data therefore no longer trip the
// filter. Wrappers (`sudo`, `env`, `nohup`, `timeout`, `xargs`, ...) are peeled
// off, and nested shells (`bash -c`, `eval`, `su -c`, `env -S`, `watch`) are
// parsed recursively.
//
// When the parser reports syntax it cannot resolve statically — a `$var` in
// command position, an unbalanced quote — the original whole-string substring
// scan runs as an explicit fallback so that policy is never silently weakened.
//
// None of this is OS isolation. It is a best-effort tripwire in front of an
// already-enabled bash tool; the container runtime is the actual boundary.

use super::legacy_scan::legacy_substring_scan;
use super::shell_parse::{
    self, FETCH_PROGRAMS, ParseBudget, Parsed, SimpleCommand, Word, basename_lower,
};

/// A denylist hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    /// Stable rule identifier (e.g. `rm-root`) or, for the fallback scan, the
    /// matched legacy pattern.
    pub rule: String,
    /// The simple command that matched, as written.
    pub site: String,
}

/// Evaluate `command` against the denylist.
pub(crate) fn check(command: &str) -> Result<(), Violation> {
    let mut ctx = Resolution::default();
    let parsed = shell_parse::parse(command, &mut ctx.budget, 0);
    ctx.unresolved.extend(parsed.unresolved);
    for cmd in &parsed.commands {
        resolve(cmd, &mut ctx, 0);
    }

    if let Some(site) = ctx.glob_commands.first() {
        return Err(Violation {
            rule: "glob-command-name".to_string(),
            site: site.clone(),
        });
    }

    check_effective(&ctx.out)?;

    if !ctx.unresolved.is_empty()
        && let Some(pattern) = legacy_substring_scan(command)
    {
        return Err(Violation {
            rule: pattern.to_string(),
            site: format!(
                "fallback scan; unresolved syntax: {}",
                ctx.unresolved.join(", ")
            ),
        });
    }
    Ok(())
}

/// Accumulated state while peeling wrappers and expanding nested scripts.
#[derive(Default)]
struct Resolution {
    out: Vec<Effective>,
    unresolved: Vec<String>,
    glob_commands: Vec<String>,
    /// Shared pipeline-id counter and work budget for nested parses.
    budget: ParseBudget,
}

/// A simple command after wrapper stripping, ready for rule evaluation.
#[derive(Debug, Clone)]
struct Effective {
    words: Vec<Word>,
    cmd: SimpleCommand,
    /// The program is a shell that will read its script from stdin.
    shell_reads_stdin: bool,
}

impl Effective {
    fn program(&self) -> String {
        self.words
            .first()
            .map(|w| basename_lower(&w.text))
            .unwrap_or_default()
    }
}

const MAX_NESTING: usize = 8;

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "ash", "mksh", "fish"];

/// Peel wrappers and expand nested shells, appending resulting commands.
fn resolve(cmd: &SimpleCommand, ctx: &mut Resolution, depth: usize) {
    if depth > MAX_NESTING {
        ctx.unresolved
            .push(format!("nesting deeper than supported in `{}`", cmd.site));
        return;
    }
    if cmd.function_def {
        ctx.out.push(Effective {
            words: cmd.words.clone(),
            cmd: cmd.clone(),
            shell_reads_stdin: false,
        });
        return;
    }

    let mut words: &[Word] = &cmd.words;
    loop {
        let Some(first) = words.first() else {
            // Only redirects (`> /dev/sda`): still needs rule evaluation.
            ctx.out.push(Effective {
                words: Vec::new(),
                cmd: cmd.clone(),
                shell_reads_stdin: false,
            });
            return;
        };
        if first.dynamic {
            // `$cmd`, `sudo $cmd`, `if $cmd`: the program is not knowable
            // statically, so the caller falls back to the substring scan.
            ctx.unresolved
                .push(format!("dynamic command name in `{}`", cmd.site));
            return;
        }
        if first.glob && !matches!(first.text.as_str(), "[" | "[[") {
            // `/sbin/reb*t`: pathname expansion could pick anything and the
            // substring fallback cannot see through the wildcard.
            ctx.glob_commands.push(cmd.site.clone());
            return;
        }
        let prog = basename_lower(&first.text);
        let rest = &words[1..];
        match prog.as_str() {
            // Shell reserved words that precede a command in the same
            // simple-command slot.
            "if" | "then" | "elif" | "else" | "do" | "while" | "until" | "!" | "coproc" => {
                words = rest;
            }
            "sudo" | "doas" => {
                let with_arg = [
                    "-u", "-g", "-p", "-C", "-D", "-h", "-r", "-t", "-T", "-U", "-a", "-c",
                ];
                // `sudo -s 'cmd'` / `sudo -i 'cmd'` run the remaining words
                // through `$SHELL -c`, so they are a script, not an argv.
                let shell_mode = option_words(rest, &with_arg).any(|t| {
                    t == "--shell"
                        || t == "--login"
                        || (t.starts_with('-') && !t.starts_with("--") && t.contains(['s', 'i']))
                });
                let after = skip_options(rest, &with_arg);
                if shell_mode {
                    if !after.is_empty() {
                        nested_script(&join_words(after), cmd, ctx, depth);
                    }
                    return;
                }
                words = after;
            }
            "env" => {
                let mut i = 0;
                while i < rest.len() {
                    let t = rest[i].text.as_str();
                    if t == "--" {
                        i += 1;
                        break;
                    } else if t == "-S" || t == "--split-string" {
                        // `env -S "cmd args"`: the string is a command line.
                        if let Some(script) = rest.get(i + 1) {
                            nested_script(script, cmd, ctx, depth);
                        }
                        return;
                    } else if let Some(s) = t.strip_prefix("--split-string=") {
                        nested_script(
                            &Word {
                                text: s.to_string(),
                                ..rest[i].clone()
                            },
                            cmd,
                            ctx,
                            depth,
                        );
                        return;
                    } else if t == "-u" || t == "-C" || t == "--unset" || t == "--chdir" {
                        i += 2;
                    } else if t.starts_with('-') || looks_like_assignment(t) {
                        i += 1;
                    } else {
                        break;
                    }
                }
                words = &rest[i.min(rest.len())..];
            }
            "nice" => words = skip_options(rest, &["-n", "--adjustment"]),
            "nohup" | "time" | "builtin" | "unbuffer" | "caffeinate" | "busybox" => {
                words = skip_options(rest, &[]);
            }
            "command" => {
                if rest.iter().any(|w| w.text == "-v" || w.text == "-V") {
                    return; // lookup only, nothing executes
                }
                words = skip_options(rest, &[]);
            }
            "exec" => words = skip_options(rest, &["-a"]),
            "setsid" => words = skip_options(rest, &[]),
            "timeout" => {
                let after = skip_options(rest, &["-s", "--signal", "-k", "--kill-after"]);
                words = after.get(1..).unwrap_or(&[]); // skip DURATION
            }
            "stdbuf" => words = skip_options(rest, &["-i", "-o", "-e"]),
            "ionice" => {
                words = skip_options(rest, &["-c", "-n", "-p", "--class", "--classdata", "--pid"])
            }
            "chroot" => {
                let after = skip_options(rest, &["--userspec", "--groups"]);
                words = after.get(1..).unwrap_or(&[]); // skip NEWROOT
            }
            "xargs" => {
                let after = skip_options(
                    rest,
                    &[
                        "-I",
                        "-n",
                        "-L",
                        "-P",
                        "-s",
                        "-d",
                        "-a",
                        "-E",
                        "--max-args",
                        "--max-procs",
                        "--max-lines",
                        "--delimiter",
                        "--arg-file",
                        "--eof",
                        "--replace",
                        "--max-chars",
                    ],
                );
                if after.is_empty() {
                    return; // default command is echo
                }
                words = after;
            }
            "watch" => {
                let after = skip_options(rest, &["-n", "--interval"]);
                nested_script(&join_words(after), cmd, ctx, depth);
                return;
            }
            "su" => {
                let mut i = 0;
                while i < rest.len() {
                    let t = rest[i].text.as_str();
                    if t == "-c" || t == "--command" || t == "--session-command" {
                        if let Some(script) = rest.get(i + 1) {
                            nested_script(script, cmd, ctx, depth);
                        }
                        return;
                    }
                    if let Some(s) = t
                        .strip_prefix("--command=")
                        .or_else(|| t.strip_prefix("--session-command="))
                    {
                        nested_script(
                            &Word {
                                text: s.to_string(),
                                ..rest[i].clone()
                            },
                            cmd,
                            ctx,
                            depth,
                        );
                        return;
                    }
                    i += if ["-s", "--shell", "-g", "--group", "-G", "--supp-group"].contains(&t) {
                        2
                    } else {
                        1
                    };
                }
                return; // interactive su: nothing static to check
            }
            "eval" => {
                nested_script(&join_words(rest), cmd, ctx, depth);
                return;
            }
            p if SHELLS.contains(&p) => {
                let (script, reads_stdin) = shell_invocation(rest);
                ctx.out.push(Effective {
                    words: words.to_vec(),
                    cmd: cmd.clone(),
                    shell_reads_stdin: reads_stdin,
                });
                if let Some(script) = script {
                    nested_script(script, cmd, ctx, depth);
                }
                return;
            }
            _ => {
                ctx.out.push(Effective {
                    words: words.to_vec(),
                    cmd: cmd.clone(),
                    shell_reads_stdin: false,
                });
                return;
            }
        }
    }
}

/// Join words into one script word (for `eval a b c`, `sudo -s a b`).
fn join_words(words: &[Word]) -> Word {
    let joined = words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Word {
        dynamic: words.iter().any(|w| w.dynamic),
        fetch_subst: words.iter().any(|w| w.fetch_subst),
        ..Word::literal(&joined)
    }
}

/// Iterate the leading option words of an argv (same stop rule as
/// `skip_options`).
fn option_words<'a>(words: &'a [Word], with_arg: &'a [&str]) -> impl Iterator<Item = &'a str> {
    let end = words.len() - skip_options(words, with_arg).len();
    words[..end].iter().map(|w| w.text.as_str())
}

/// Parse the argument of `bash -c` / `eval` as a nested command line.
fn nested_script(script: &Word, parent: &SimpleCommand, ctx: &mut Resolution, depth: usize) {
    if script.dynamic {
        ctx.unresolved
            .push(format!("dynamic script argument in `{}`", parent.site));
        return;
    }
    if ctx.budget.remaining == 0 {
        ctx.unresolved.push("work budget exceeded".to_string());
        return;
    }
    let parsed: Parsed = shell_parse::parse(&script.text, &mut ctx.budget, depth + 1);
    ctx.unresolved.extend(parsed.unresolved);
    for cmd in &parsed.commands {
        resolve(cmd, ctx, depth + 1);
    }
}

/// Skip leading option words. Options listed in `with_arg` consume the next
/// word too. Stops at `--` (consumed) or the first non-option word.
fn skip_options<'a>(words: &'a [Word], with_arg: &[&str]) -> &'a [Word] {
    let mut i = 0;
    while i < words.len() {
        let t = words[i].text.as_str();
        if t == "--" {
            return &words[i + 1..];
        }
        if with_arg.contains(&t) {
            i += 2;
        } else if t.starts_with('-') && t.len() > 1 {
            i += 1;
        } else {
            break;
        }
    }
    &words[i.min(words.len())..]
}

/// Interpret shell arguments: returns the `-c` script word if any, and whether
/// the shell will read its script from stdin.
fn shell_invocation(rest: &[Word]) -> (Option<&Word>, bool) {
    let mut has_c = false;
    let mut has_s = false;
    let mut i = 0;
    while i < rest.len() {
        let t = rest[i].text.as_str();
        if t == "--" {
            i += 1;
            break;
        }
        if ["-o", "+o", "-O", "+O", "--rcfile", "--init-file"].contains(&t) {
            i += 2;
            continue;
        }
        if t.starts_with("--") {
            i += 1;
            continue;
        }
        if (t.starts_with('-') || t.starts_with('+')) && t.len() > 1 {
            if t.starts_with('-') && t.contains('c') {
                has_c = true;
            }
            if t.starts_with('-') && t.contains('s') {
                has_s = true;
            }
            i += 1;
            continue;
        }
        break;
    }
    if has_c {
        return (rest.get(i), false);
    }
    // No -c: a positional word is a script file; none (or `-`) means stdin.
    let positional = rest.get(i).map(|w| w.text.as_str());
    (None, has_s || matches!(positional, None | Some("-")))
}

fn looks_like_assignment(t: &str) -> bool {
    match t.find('=') {
        Some(eq) if eq > 0 => t[..eq]
            .strip_suffix('+')
            .unwrap_or(&t[..eq])
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

fn violation(rule: &str, cmd: &SimpleCommand) -> Violation {
    Violation {
        rule: rule.to_string(),
        site: cmd.site.clone(),
    }
}

fn check_effective(cmds: &[Effective]) -> Result<(), Violation> {
    let function_names: Vec<String> = cmds
        .iter()
        .filter(|e| e.cmd.function_def)
        .filter_map(|e| e.words.first().map(|w| w.text.clone()))
        .collect();

    for (idx, e) in cmds.iter().enumerate() {
        let cmd = &e.cmd;

        // Writes to raw block devices via redirection.
        for r in &cmd.redirects {
            if r.writes_target() && is_block_device(r.target.static_prefix()) {
                return Err(violation("block-device-write", cmd));
            }
        }

        if cmd.function_def {
            continue;
        }
        let prog = e.program();
        if prog.is_empty() {
            continue;
        }
        let args = Args::parse(&e.words[1..]);

        match prog.as_str() {
            "rm" => {
                let recursive =
                    args.has_short('r') || args.has_short('R') || args.has_long("recursive");
                if recursive
                    && args
                        .positionals
                        .iter()
                        .any(|w| is_root_target(w.static_prefix()))
                {
                    return Err(violation("rm-root", cmd));
                }
                if args.has_long("no-preserve-root") {
                    return Err(violation("rm-no-preserve-root", cmd));
                }
            }
            p if p.starts_with("mkfs") || p == "mke2fs" || p == "mkswap" => {
                return Err(violation("mkfs", cmd));
            }
            "dd" => {
                for w in &args.positionals {
                    if let Some(src) = w.text.strip_prefix("if=")
                        && matches!(src, "/dev/zero" | "/dev/random" | "/dev/urandom")
                    {
                        return Err(violation("dd-device-source", cmd));
                    }
                    if let Some(dst) = w.static_prefix().strip_prefix("of=")
                        && is_block_device(dst)
                    {
                        return Err(violation("dd-block-device", cmd));
                    }
                }
            }
            "shutdown" | "reboot" | "halt" | "poweroff" => {
                return Err(violation("power-state", cmd));
            }
            "systemctl" => {
                if args
                    .positionals
                    .iter()
                    .any(|w| matches!(w.text.as_str(), "reboot" | "poweroff" | "halt" | "kexec"))
                {
                    return Err(violation("power-state", cmd));
                }
            }
            "init" | "telinit" => {
                if args
                    .positionals
                    .iter()
                    .any(|w| w.text == "0" || w.text == "6")
                {
                    return Err(violation("power-state", cmd));
                }
            }
            "chmod" => {
                let recursive = args.has_short('R') || args.has_long("recursive");
                if recursive
                    && args
                        .positionals
                        .iter()
                        .skip(1)
                        .any(|w| is_root_target(w.static_prefix()))
                {
                    return Err(violation("chmod-root", cmd));
                }
            }
            "chown" => {
                let recursive = args.has_short('R') || args.has_long("recursive");
                if recursive {
                    let owner = args
                        .positionals
                        .first()
                        .map(|w| w.text.as_str())
                        .unwrap_or("");
                    let root_owner = owner == "root"
                        || owner.starts_with("root:")
                        || owner == "0"
                        || owner.starts_with("0:");
                    // Not `skip(1)`: in `chown -Rroot /` the owner is glued to
                    // the flag cluster and `/` is the only positional.
                    let root_target = args
                        .positionals
                        .iter()
                        .any(|w| is_root_target(w.static_prefix()));
                    if root_owner || root_target {
                        return Err(violation("chown-root", cmd));
                    }
                }
            }
            p if SHELLS.contains(&p) => {
                // `bash <(curl …)`, `sh -c "$(curl …)"`, `bash < <(curl …)`,
                // `bash <<< "$(curl …)"`.
                if e.words.iter().skip(1).any(|w| w.fetch_subst)
                    || cmd.redirects.iter().any(|r| r.target.fetch_subst)
                {
                    return Err(violation("fetch-to-shell", cmd));
                }
                if e.shell_reads_stdin {
                    let upstream = || cmds[..idx].iter().filter(|prev| upstream_of(prev, cmd));
                    // `curl … | sh`: any earlier command in the same pipeline fetches.
                    if upstream().any(|prev| FETCH_PROGRAMS.contains(&prev.program().as_str())) {
                        return Err(violation("fetch-to-shell", cmd));
                    }
                    // `bash <<EOF … EOF`, `bash <<< "…"`, `cat <<EOF … EOF | sh`:
                    // the shell executes heredoc / here-string data.
                    let scripts = cmd
                        .redirects
                        .iter()
                        .chain(upstream().flat_map(|prev| prev.cmd.redirects.iter()))
                        .filter(|r| matches!(r.op.as_str(), "<<" | "<<<"))
                        .map(|r| r.target.text.clone())
                        .collect::<Vec<_>>();
                    for script in scripts {
                        if let Err(inner) = check(&script) {
                            return Err(Violation {
                                rule: inner.rule,
                                site: format!("{} (script on stdin)", cmd.site),
                            });
                        }
                    }
                }
            }
            _ => {}
        }

        // Fork bomb: a function that pipes itself into itself.
        if function_names.iter().any(|n| n == &e.words[0].text)
            && cmds.get(idx + 1).is_some_and(|next| {
                next.cmd.pipeline == cmd.pipeline
                    && next.cmd.pipe_index == cmd.pipe_index + 1
                    && next.words.first().map(|w| w.text.as_str()) == Some(e.words[0].text.as_str())
            })
        {
            return Err(violation("fork-bomb", cmd));
        }
    }
    Ok(())
}

/// Is `prev` (which appears earlier in source order) an earlier stage of the
/// pipeline feeding `cmd`? Direct stages share a pipeline id; commands inside
/// a `(…)` / `{…}` group that is itself a stage record the outer pipeline in
/// `enclosing`, in either direction (`(curl x) | sh`, `curl x | (sh)`).
fn upstream_of(prev: &Effective, cmd: &SimpleCommand) -> bool {
    let p = &prev.cmd;
    (p.pipeline == cmd.pipeline && p.pipe_index < cmd.pipe_index)
        || p.enclosing.contains(&cmd.pipeline)
        || cmd.enclosing.contains(&p.pipeline)
        || p.enclosing.iter().any(|e| cmd.enclosing.contains(e))
}

/// Parsed argument list: short flag letters, long flags, positionals.
struct Args<'a> {
    shorts: String,
    longs: Vec<String>,
    positionals: Vec<&'a Word>,
}

impl<'a> Args<'a> {
    fn parse(words: &'a [Word]) -> Self {
        let mut a = Args {
            shorts: String::new(),
            longs: Vec::new(),
            positionals: Vec::new(),
        };
        let mut opts_done = false;
        for w in words {
            let t = w.text.as_str();
            // Flags are read from the literal prefix so `-rf$x` still counts
            // as recursive.
            let t = if w.dynamic { w.static_prefix() } else { t };
            if opts_done || t.is_empty() {
                a.positionals.push(w);
            } else if t == "--" {
                opts_done = true;
            } else if let Some(l) = t.strip_prefix("--") {
                a.longs.push(l.split('=').next().unwrap_or(l).to_string());
            } else if t.len() > 1 && t.starts_with('-') {
                a.shorts.push_str(&t[1..]);
            } else {
                a.positionals.push(w);
            }
        }
        a
    }

    fn has_short(&self, c: char) -> bool {
        self.shorts.contains(c)
    }

    fn has_long(&self, name: &str) -> bool {
        self.longs.iter().any(|l| l == name)
    }
}

/// `/`, `//`, `/*`, `/.`, `/.*`, `/[a-z]*` and friends: an absolute path with
/// no literal path component, so it names the root or everything under it.
fn is_root_target(t: &str) -> bool {
    if !t.starts_with('/') {
        return false;
    }
    let mut in_bracket = false;
    for c in t.chars() {
        match c {
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            '/' | '*' | '?' | '.' => {}
            _ if in_bracket => {}
            _ => return false,
        }
    }
    true
}

fn is_block_device(t: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "/dev/sd",
        "/dev/hd",
        "/dev/vd",
        "/dev/xvd",
        "/dev/nvme",
        "/dev/mmcblk",
        "/dev/disk",
        "/dev/md",
        "/dev/dm-",
        "/dev/mapper/",
        "/dev/loop",
    ];
    PREFIXES.iter().any(|p| {
        t.strip_prefix(p)
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    })
}
