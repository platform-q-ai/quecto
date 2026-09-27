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
    /// A standard asset's path holding exactly bytes an earlier quecto
    /// shipped (#2206). Only `container status`, `container doctor` and
    /// `container init` rewrite it; a launch and a teardown refuse it, since
    /// an older quecto still running may be using those very scripts.
    Outdated,
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
        let project = bundle_project(script);
        let script = script.display();
        match self {
            Self::NotStandard | Self::Intact => None,
            Self::Differs => Some(format!(
                "{script} differs from the standard bundle this quecto embeds (or this quecto embeds a newer bundle than the one that wrote it — run `quecto container init --refresh`); it is a host-side script the launch would run before any container exists, so review what changed in it (the scripts are usually not tracked by git: compare the file with the bundle a fresh `quecto container init` writes in an empty directory) and restore the bundle with `quecto container init --refresh` (or delete the file and run `quecto container init`)"
            )),
            Self::Outdated => Some(match project {
                Some(project) => {
                    let project = project.display();
                    format!(
                        "{script} was written by an earlier quecto; run `quecto container status --project {project}` (or `quecto container init --refresh --project {project}`) to update it — restart any older quecto that is still running first"
                    )
                }
                None => format!(
                    "{script} was written by an earlier quecto; run `quecto container status` (or `quecto container init --refresh`) in its project to update it — restart any older quecto that is still running first"
                ),
            }),
            Self::Missing => Some(format!(
                "{script} is missing from the standard bundle; run `quecto container init` to materialise it"
            )),
            Self::Refused(reason) => Some(format!(
                "{script} cannot be judged: {reason}; restore a regular file there and run `quecto container init --refresh`"
            )),
        }
    }
}

/// The bundle's directory below a project, as `init` lays it out.
const BUNDLE_DIR: &str = ".quecto/containers/standard";

/// The project a bundle script belongs to: the directory holding the
/// `.quecto/containers/standard` it lies in (#2206 round 4, so a refusal
/// can name where to run `container status`).
fn bundle_project(script: &std::path::Path) -> Option<&std::path::Path> {
    let bundle = script.ancestors().find(|dir| dir.ends_with(BUNDLE_DIR))?;
    bundle
        .ancestors()
        .nth(std::path::Path::new(BUNDLE_DIR).components().count())
}
