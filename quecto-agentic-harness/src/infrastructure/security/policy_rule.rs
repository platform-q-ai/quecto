// The command-policy rules (#1620) and how each explains a refusal (#2198).
//
// A refusal carries the rule's stable id for the logs, and for the model a
// plain reason and a way forward. The match below is exhaustive, so a new
// rule cannot be added without its explanation.

/// A rule of the dangerous-command denylist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyRule {
    GlobCommandName,
    BlockDeviceWrite,
    RmRoot,
    RmNoPreserveRoot,
    RmProtectedDir,
    Mkfs,
    DdDeviceSource,
    DdBlockDevice,
    PowerState,
    ChmodRoot,
    ChownRoot,
    FetchToShell,
    ForkBomb,
    /// The whole-string fallback scan found this legacy pattern in a
    /// command whose syntax the parser could not resolve.
    Fallback(&'static str),
}

impl PolicyRule {
    /// The stable id, as logged and documented in `docs/command-policy.md`.
    pub fn id(self) -> &'static str {
        match self {
            Self::GlobCommandName => "glob-command-name",
            Self::BlockDeviceWrite => "block-device-write",
            Self::RmRoot => "rm-root",
            Self::RmNoPreserveRoot => "rm-no-preserve-root",
            Self::RmProtectedDir => "rm-protected-dir",
            Self::Mkfs => "mkfs",
            Self::DdDeviceSource => "dd-device-source",
            Self::DdBlockDevice => "dd-block-device",
            Self::PowerState => "power-state",
            Self::ChmodRoot => "chmod-root",
            Self::ChownRoot => "chown-root",
            Self::FetchToShell => "fetch-to-shell",
            Self::ForkBomb => "fork-bomb",
            Self::Fallback(_) => "fallback-scan",
        }
    }

    /// What the rule refuses and why, in plain words (no final full stop).
    pub fn reason(self) -> String {
        let reason = match self {
            Self::GlobCommandName => {
                "a glob in the program name (`*`, `?` or `[`) could expand to any program, so \
                 the policy cannot tell what would run"
            }
            Self::BlockDeviceWrite => {
                "an output redirect onto a raw block device (e.g. `> /dev/sda`) overwrites a disk"
            }
            Self::RmRoot => {
                "a recursive delete of the filesystem root (`/`, `/*`) would destroy the whole \
                 system: every program, configuration file and user's data"
            }
            Self::RmNoPreserveRoot => {
                "`rm --no-preserve-root` turns off rm's own guard against deleting `/`"
            }
            Self::RmProtectedDir => {
                "a recursive delete of the home directory, the workspace root, another user's \
                 home or a top-level directory (e.g. `/etc`, `/tmp`) would destroy a user's \
                 files, the whole project or part of the system"
            }
            Self::Mkfs => {
                "`mkfs*`, `mke2fs` and `mkswap` do not run here in any form, even `--version` or \
                 `--help`: they erase whatever device they format"
            }
            Self::DdDeviceSource => {
                "`dd` reading `/dev/zero`, `/dev/random` or `/dev/urandom` is refused: it is the \
                 usual way to wipe a disk, and one wrong `of=` erases it"
            }
            Self::DdBlockDevice => {
                "`dd` writing to a raw block device (`of=/dev/sd*`, …) overwrites a disk"
            }
            Self::PowerState => {
                "`shutdown`, `reboot`, `halt` and `poweroff` do not run here in any form, nor do \
                 `systemctl reboot`/`poweroff`/`halt`/`kexec` or `init 0`/`6`: they stop or \
                 restart the machine and everything running on it"
            }
            Self::ChmodRoot => {
                "a recursive `chmod` of the filesystem root would change every file's permissions, \
                 breaking the system's programs and its security"
            }
            Self::ChownRoot => {
                "a recursive `chown` to root, or of the filesystem root, would take files from \
                 their owners or break the system's programs"
            }
            Self::FetchToShell => {
                "a downloaded script fed straight into a shell (e.g. `curl … | sh`) runs unread"
            }
            Self::ForkBomb => {
                "a function that pipes itself into itself (a fork bomb) exhausts the machine's \
                 processes"
            }
            Self::Fallback(pattern) => {
                return format!(
                    "the command has syntax the policy cannot resolve before it runs, so it was \
                     scanned as text, and the text contains `{pattern}`"
                );
            }
        };
        reason.to_string()
    }

    /// What to do instead (no final full stop): a narrower command the
    /// task may need, never another way to the refused effect.
    pub fn instead(self) -> String {
        const NOTHING: &str = "none: no form of it runs here";
        let instead = match self {
            Self::GlobCommandName => {
                "name the program literally (e.g. `/bin/echo`, not `/bin/ech?`) so the policy can \
                 check it; globs in the arguments are fine"
            }
            Self::BlockDeviceWrite => "write to a regular file (e.g. `> disk.img`)",
            Self::RmRoot | Self::ChmodRoot => {
                "name only the specific paths the task needs, below the top level (e.g. \
                 `/tmp/build`)"
            }
            Self::RmNoPreserveRoot => {
                "drop `--no-preserve-root` and name only the specific paths the task needs"
            }
            Self::RmProtectedDir => {
                "delete only the specific paths the task needs below it (e.g. `./target`, \
                 `/tmp/build`)"
            }
            Self::DdBlockDevice => "write to a regular file (e.g. `of=disk.img`)",
            Self::ChownRoot => {
                "change the owner of only the specific paths the task needs, to a non-root user"
            }
            Self::FetchToShell => {
                "download it to a file and read it, then tell the user what it would do"
            }
            Self::DdDeviceSource => {
                "to fill a regular file, use `head -c <size> /dev/zero > file` (or \
                 `/dev/urandom`), or `truncate -s <size> file`"
            }
            Self::Mkfs | Self::PowerState | Self::ForkBomb => NOTHING,
            Self::Fallback(_) => {
                "if the words are only data, write the `$…` part (program or script) literally \
                 so the policy can parse it; if the command would really run it, stop and tell \
                 the user"
            }
        };
        instead.to_string()
    }

    /// The last word of a refusal, a whole sentence: the effect is not to be
    /// had another way.
    pub fn closing(self) -> &'static str {
        match self {
            // A literal program name is checked by the same rules.
            Self::GlobCommandName | Self::Fallback(_) => {
                "Written so the policy can read it, the command is checked again; getting a \
                 refused effect another way is not allowed; if the task needs it, tell the user."
            }
            Self::BlockDeviceWrite
            | Self::RmRoot
            | Self::RmNoPreserveRoot
            | Self::RmProtectedDir
            | Self::Mkfs
            | Self::DdDeviceSource
            | Self::DdBlockDevice
            | Self::PowerState
            | Self::ChmodRoot
            | Self::ChownRoot
            | Self::FetchToShell
            | Self::ForkBomb => {
                "Getting the same effect another way is not allowed; if the task needs it, tell \
                  the user."
            }
        }
    }
}
