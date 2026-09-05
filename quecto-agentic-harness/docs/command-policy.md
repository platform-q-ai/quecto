# Command policy: the dangerous-command denylist

The `bash` tool runs every command through a denylist before it spawns a
shell. This page describes what the denylist does, how it reads a command,
what it deliberately does not do, and what changed in #1620.

## This is not OS isolation

Command filtering is a tripwire in front of an already-enabled tool. It
inspects the *text* of a command; it does not confine the process, the
filesystem or the network. Anything the shell can reach, an agent with
`bash` can reach, and a determined model can express most destructive
operations in a form the filter does not recognise (a script written to disk
and then executed, a path that arrives on stdin, an interpreter one-liner).

For untrusted work, the boundary is the container runtime described in
[`docs/container-runtimes.md`](../../docs/container-runtimes.md) and the
tool-level policy in [`tool-policy.md`](tool-policy.md), which decides whether
`bash` is available to a given agent at all. The denylist exists so that a
harness running directly on a developer workstation does not execute
`rm -rf /` or `reboot` by accident.

## Execution-aware matching

Before #1620 the denylist was a substring scan over the whole command
string. `echo "the box will reboot"`, `grep halt notes.md`,
`cat docs/shutdown-procedure.md` and any heredoc that mentioned a dangerous
command were all blocked, and a quote-unaware tokenizer mistook punctuation
inside arguments for shell syntax.

The filter now parses the command into simple commands first and matches
rules against the **execution site**: the program word of each simple
command, its arguments and its redirections. The parser understands:

- single, double and ANSI-C (`$'...'`) quoting, backslash escapes and line
  continuations;
- `;`, `&&`, `||`, `|`, `&` and newlines as command separators, with
  pipeline membership tracked;
- command substitution (`$(...)`, backticks) and process substitution
  (`<(...)`, `>(...)`), whose contents are parsed as nested commands;
- redirections (`>`, `>>`, `>|`, `&>`, `2>&1`, `<`, `<<<`) as operators
  separate from argv;
- heredocs (`<<EOF`, `<<-EOF`, `<<'EOF'`), whose bodies are data and never
  matched;
- leading `NAME=value` assignments, comments, subshell and brace groups,
  `name()` and `function name` definitions;
- brace expansion (`{rm,} -rf /`, `rm -rf /{,tmp}`), expanded into the
  simple commands bash would run, bounded to 64 variants per command;
- shell keywords in command position (`if reboot; then`, `! halt`,
  `while`, `until`, `do`, `coproc`), which are skipped so the command after
  them is evaluated.

On top of the parse, the rule engine peels wrappers and looks inside
nested shells:

| Construct | Handling |
| --- | --- |
| `sudo`, `doas`, `env`, `nice`, `nohup`, `time`, `command`, `builtin`, `exec`, `setsid`, `timeout`, `stdbuf`, `ionice`, `chroot`, `xargs`, `busybox`, `unbuffer` | Options are skipped and the wrapped command is evaluated. `command -v` and a bare `xargs` execute nothing. |
| `bash -c`, `sh -c`, `zsh -c`, `dash -c`, `ksh -c`, `env -S`, `su -c`, `watch`, `eval` | The script argument is parsed recursively. |
| `curl … \| sh`, `bash <(curl …)`, `sh -c "$(curl …)"` | Pipeline and substitution structure is used to detect a fetched script feeding a shell that reads it. |
| `bash <<EOF … EOF`, `bash <<< "…"`, `cat <<EOF … EOF \| sh` | Heredoc and here-string bodies reaching a shell that reads stdin are evaluated as scripts. |

### Rules

| Rule id | Blocks |
| --- | --- |
| `rm-root` | `rm` with a recursive flag and a root target (`/`, `/*`, `//`, `/.`). Absolute paths below root are allowed. |
| `rm-no-preserve-root` | any `rm --no-preserve-root` |
| `mkfs` | `mkfs`, `mkfs.*`, `mke2fs`, `mkswap` |
| `dd-device-source` | `dd if=/dev/zero`, `/dev/random`, `/dev/urandom` |
| `dd-block-device` | `dd of=` a raw block device |
| `power-state` | `shutdown`, `reboot`, `halt`, `poweroff`, `systemctl reboot/poweroff/halt/kexec`, `init 0`, `init 6`, `telinit 0/6` |
| `chmod-root` | recursive `chmod` on `/` |
| `chown-root` | recursive `chown` to `root`/`0`, or recursive `chown` on `/` |
| `block-device-write` | an unquoted output redirect onto `/dev/sd*`, `/dev/nvme*`, `/dev/mmcblk*`, `/dev/md*`, `/dev/mapper/*`, `/dev/loop*` and similar |
| `fork-bomb` | a function that pipes itself into itself (`:(){ :\|:& };:`) |
| `fetch-to-shell` | `curl`/`wget`/`fetch`/`aria2c` output piped into a shell that reads stdin, or fed to a shell through a substitution |
| `glob-command-name` | an unquoted `*`, `?` or `[` in the program word (`/sbin/reb*t`). Pathname expansion could resolve it to anything and the fallback scan cannot see through the wildcard, so it is rejected outright. |

Program names are matched on their basename, case-insensitively, so
`/sbin/reboot` and `ReBoOt` both match. Quoting the program name does not
help: `'re'boot` and `$'\x72\x6d' -rf /` are recognised after quote removal.

### Error reporting

A rejection names the rule and the simple command it matched at:

```
command 'echo start; sudo rm -rf / ; echo end' matches dangerous pattern 'rm-root' at `sudo rm -rf /`
```

## Explicit fallback for unresolved syntax

Some constructs cannot be resolved statically: a parameter expansion in
command position (`cmd='rm -rf /'; $cmd`, `$CARGO build`), an unbalanced
quote, or nesting deeper than the parser follows. In those cases the filter
does not guess. It runs the pre-#1620 whole-string substring scan over the
original command and reports the match as a fallback:

```
command 'cmd='rm -rf /'; $cmd' matches dangerous pattern 'rm -rf /' at `fallback scan; unresolved syntax: dynamic command name in `$cmd``
```

This keeps every protection the old scan provided for dynamic commands at
the cost of the old false positives, but only when dynamic syntax is
present. `msg='please reboot'; $ECHO "$msg"` is still rejected, for example.
Writing the program name literally avoids the fallback.

A shell reading a script from stdin reports the nested rule with the shell as
the site, for example `bash <<EOF … EOF` → `'rm-root' at \`bash (script on stdin)\``.

## Known gaps

These were not caught before #1620 either. They are listed so nobody
mistakes the filter for a boundary.

- **Targets that arrive as data.** `echo / | xargs rm -rf` and
  `find / -delete` carry the root path on stdin or in a `find` predicate,
  not in `rm`'s argv.
- **Dynamic arguments.** `rm -rf "$EMPTY"/` expands to `rm -rf /` at
  runtime. Only dynamic *command names* trigger the fallback scan.
- **Interpreters.** `python -c "import shutil; shutil.rmtree('/')"` is a
  source snippet to the filter, and deliberately so.
- **Indirection through the filesystem.** Writing a script and executing it
  in a later tool call.
- **Remote hosts.** `ssh host reboot` is not inspected.

## Compatibility changes in #1620

- **The command allowlist is gone.** `agents.defaults.command_allowlist`
  is still accepted by the config loader so existing files keep loading, but
  it is ignored and a warning is logged at startup. Restrict what an agent
  can run with tool policy and the container runtime instead.
- **`rm -rf /some/absolute/path` is now allowed.** The old substring scan
  blocked any recursive delete of an absolute path because the string
  contained `rm -rf /`. Only root targets are blocked now.
- **Prose, filenames, source snippets and heredocs no longer match.**
- **Some rules are broader than before** because they match structure
  rather than one spelling: `rm -r /` without `-f`, `sudo reboot`,
  `bash -c 'reboot'`, `curl -fsSL url | bash`, writes to any raw block
  device, and `systemctl poweroff` are all now blocked.
- **Error messages changed shape.** They still contain the phrase
  `dangerous pattern`, and now also carry the rule id and the matched site.
- **Globs in the program word are rejected outright.** `/usr/bin/l?` used
  to pass; it is now blocked because it cannot be checked.
