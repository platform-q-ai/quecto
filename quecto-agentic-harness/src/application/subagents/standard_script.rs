//! The standard bundle's verdict on a host-side script (#2024 S4e): the
//! type the [`ContainerScriptIntegrity`](super::ports::ContainerScriptIntegrity)
//! port answers with, and the one refusal text every path that judges a
//! script prints.

/// The standard bundle's verdict on a script a container config's argv
/// names (#2024 S4e): whether it is one of the bundle's materialised
/// host-side scripts and, if so, whether it still carries the bytes this
/// binary embeds. The scripts run on the host before any container
/// exists, so a launch trusts them only while they are the bundle's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StandardScriptVerdict {
    /// Not a standard-bundle asset: the entry brought its own script,
    /// vouched for by the configuration's trust alone.
    NotStandard,
    /// The bundle's bytes exactly.
    Intact,
    /// A standard asset's path holding other bytes (an edit, a pull,
    /// another version).
    Differs,
    /// A standard asset's path with nothing there.
    Missing,
    /// A standard asset's path that cannot be judged (a symbolic link in
    /// its place or on the way); the reason.
    Refused(String),
}

impl StandardScriptVerdict {
    /// Why `script` must not run, for a verdict that refuses it; `None`
    /// for a script the launch may run (the bundle's bytes exactly, or
    /// not the bundle's at all). One text for every path that judges a
    /// script: the create, a join, and the retained inspect/kill/cleanup.
    pub fn refusal(&self, script: &std::path::Path) -> Option<String> {
        let script = script.display();
        match self {
            Self::NotStandard | Self::Intact => None,
            Self::Differs => Some(format!(
                "{script} differs from the standard bundle this quecto embeds (or this quecto embeds a newer bundle than the one that wrote it — run `quecto container init --refresh`); it is a host-side script the launch would run before any container exists, so review the change (git diff) and restore the bundle with `quecto container init --refresh` (or delete the file and run `quecto container init`)"
            )),
            Self::Missing => Some(format!(
                "{script} is missing from the standard bundle; run `quecto container init` to materialise it"
            )),
            Self::Refused(reason) => Some(format!(
                "{script} cannot be judged: {reason}; restore a regular file there and run `quecto container init --refresh`"
            )),
        }
    }
}
