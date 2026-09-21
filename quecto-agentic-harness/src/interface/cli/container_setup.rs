//! `quecto container init` and `quecto container status` (#2024 S4e):
//! parse the arguments, invoke the composed use case for the project
//! (the working directory unless `--project` names another), present
//! the report. `init` prints the files it materialised, the entry it
//! wrote and where, and the exact build command that is the one step
//! left (the bundle's own, so no runtime is named here); `status` prints one line each for the assets, the entry,
//! the trust and the image (asked of the entry's own create preflight)
//! and exits 1 while anything is missing. The interface resolves no
//! config, reads no file and runs no script itself.

use std::path::PathBuf;

use super::CliContext;
use crate::application::configuration::dto::ConfigSelection;
use crate::application::environments::dto::{
    AssetState, CheckStatus, ContainerConfigLayer, EntryValueChange, RepositoryOrigin,
    StandardContainerReport, StandardContainerRequest, StandardContainerStatus, StandardDefault,
};
use crate::domain::redaction::redact_url_userinfo;

const INIT_USAGE: &str = "usage: quecto container init [--project <abs dir>] [--repo <url>] [--image <tag>] [--refresh] [--dry-run]\n";
const STATUS_USAGE: &str = "usage: quecto container status [--project <abs dir>]\n";

struct InitArgs {
    project: Option<PathBuf>,
    repository: Option<String>,
    image: Option<String>,
    dry_run: bool,
    refresh: bool,
}

fn parse_init(args: &[String]) -> Result<InitArgs, String> {
    let mut parsed = InitArgs {
        project: None,
        repository: None,
        image: None,
        dry_run: false,
        refresh: false,
    };
    let mut rest = args.iter();
    let value = |flag: &str, rest: &mut std::slice::Iter<String>| {
        rest.next()
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| format!("{flag} requires a value\n{INIT_USAGE}"))
    };
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--project" => {
                let project = PathBuf::from(value("--project", &mut rest)?);
                if !project.is_absolute() {
                    return Err(format!(
                        "--project must be an absolute path: {}\n{INIT_USAGE}",
                        project.display()
                    ));
                }
                parsed.project = Some(project);
            }
            "--repo" => parsed.repository = Some(value("--repo", &mut rest)?),
            "--image" => parsed.image = Some(value("--image", &mut rest)?),
            "--dry-run" => parsed.dry_run = true,
            "--refresh" => parsed.refresh = true,
            other => return Err(format!("unknown argument {other}\n{INIT_USAGE}")),
        }
    }
    Ok(parsed)
}

fn parse_status(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut project = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--project" => {
                let path = PathBuf::from(
                    rest.next()
                        .filter(|v| !v.is_empty())
                        .ok_or_else(|| format!("--project requires a value\n{STATUS_USAGE}"))?,
                );
                if !path.is_absolute() {
                    return Err(format!(
                        "--project must be an absolute path: {}\n{STATUS_USAGE}",
                        path.display()
                    ));
                }
                project = Some(path);
            }
            other => return Err(format!("unknown argument {other}\n{STATUS_USAGE}")),
        }
    }
    Ok(project)
}

/// The project the command works on, and the configuration selection its
/// overlay belongs to: `--project` or the working directory, canonical.
/// A `--project` elsewhere than the working directory selects THAT
/// directory's layers, so the overlay written is the project's.
fn project_selection(
    ctx: &CliContext,
    project: Option<PathBuf>,
) -> Result<(PathBuf, ConfigSelection), String> {
    if ctx.config_path.is_some() {
        return Err(
            "container init/status work on the project's repo-local overlay, which an explicit --config replaces; run without --config".to_string(),
        );
    }
    let project = match project.or_else(|| ctx.cwd.clone()) {
        Some(path) => path,
        None => return Err("the working directory is unknown; pass --project".to_string()),
    };
    let project = std::fs::canonicalize(&project)
        .map_err(|error| format!("project {} is not accessible: {error}", project.display()))?;
    let mut for_project = ctx.clone();
    for_project.cwd = Some(project.clone());
    let selection = for_project.config_selection()?;
    Ok((project, selection))
}

/// The base directory as an absolute path: the state dir the entry's
/// argv names must not depend on the cwd a later create runs in.
fn absolute_base_dir(ctx: &CliContext) -> Result<PathBuf, String> {
    let base_dir = ctx.base_dir();
    match std::fs::canonicalize(&base_dir) {
        Ok(path) => Ok(path),
        Err(_) if base_dir.is_absolute() => Ok(base_dir),
        Err(_) => std::env::current_dir()
            .map(|cwd| cwd.join(&base_dir))
            .map_err(|error| format!("cannot resolve base dir {}: {error}", base_dir.display())),
    }
}

pub(crate) fn cmd_init(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = parse_init(args).and_then(|parsed| {
        let Some(build) = ctx.container_init else {
            return Err("container init not composed".to_string());
        };
        let (project, selection) = project_selection(ctx, parsed.project)?;
        build(&absolute_base_dir(ctx)?, &selection)
            .execute(&StandardContainerRequest {
                project,
                repository: parsed.repository,
                image: parsed.image,
                dry_run: parsed.dry_run,
                refresh: parsed.refresh,
            })
            .map_err(|error| error.to_string())
    });
    match outcome {
        Ok(report) => {
            present_init(&report, stdout);
            0
        }
        Err(error) => {
            stderr.push_str(&error);
            if !error.ends_with('\n') {
                stderr.push('\n');
            }
            1
        }
    }
}

fn present_init(report: &StandardContainerReport, out: &mut String) {
    let verb = if report.dry_run {
        "would write"
    } else {
        "wrote"
    };
    out.push_str(&format!(
        "standard container bundle (version {}) at {}\n",
        report.version,
        report.assets_dir.display()
    ));
    for path in &report.written {
        out.push_str(&format!("  {verb}  {}\n", path.display()));
    }
    for path in &report.kept {
        out.push_str(&format!("  kept   {}\n", path.display()));
    }
    for path in &report.refreshed {
        out.push_str(&format!(
            "  {}  {} (replaced with the embedded version {})\n",
            if report.dry_run {
                "would refresh"
            } else {
                "refreshed"
            },
            path.display(),
            report.version
        ));
    }
    for path in &report.own {
        out.push_str(&format!(
            "  kept   {} (this project's own; never replaced — review it before building)\n",
            path.display()
        ));
    }
    for path in &report.differing {
        out.push_str(&format!(
            "  kept   {} (differs from the embedded version {}; a launch refuses it — review the change, then `quecto container init --refresh` restores it)\n",
            path.display(),
            report.version
        ));
    }
    if report.written.is_empty() && report.refreshed.is_empty() && !report.dry_run {
        out.push_str(
            "  no files changed (a script is replaced only by --refresh; the project's Containerfile never)\n",
        );
    }
    let entry = &report.entry;
    let location = entry
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "the repo-local overlay".to_string());
    out.push_str(&format!(
        "container config \"{}\" {} container_configs.{} in {location}{}\n",
        entry.name,
        if report.dry_run {
            "would be written as"
        } else {
            "written as"
        },
        entry.name,
        if report.dry_run {
            ""
        } else {
            " (trusted for exactly these bytes)"
        }
    ));
    out.push_str(
        "  default: true — this repo's default: `spawn container: true` selects it in this repo (no default elsewhere overrides a repo's standard container)\n",
    );
    if let Some(displaced) = &entry.displaced_default {
        out.push_str(&format!(
            "  displaced default: {displaced} (the overlay entry {} its \"default\": true label in the same write; select it by name with container: {{\"mode\":\"new\",\"container_config\":\"{displaced}\"}}; to make it the default again after removing standard: quecto config set --local container_configs.{displaced}.default true)\n",
            if report.dry_run { "would lose" } else { "lost" }
        ));
    }
    if let Some(global) = &entry.overridden_global_default {
        out.push_str(&format!(
            "  the global default {global} does not apply in this repo (the global file is untouched; other repos keep it)\n"
        ));
    }
    match (&report.repository, &report.repository_origin) {
        (Some(url), RepositoryOrigin::Explicit) => out.push_str(&format!(
            "  --repo {} (as given): a new container is a fresh clone of it\n",
            redact_url_userinfo(url)
        )),
        (Some(url), RepositoryOrigin::ExistingEntry) => out.push_str(&format!(
            "  --repo {} (the existing entry's): a new container is a fresh clone of it\n",
            redact_url_userinfo(url)
        )),
        (Some(url), _) => out.push_str(&format!(
            "  --repo {} (the checkout's origin remote): a new container is a fresh clone of it\n",
            redact_url_userinfo(url)
        )),
        (None, RepositoryOrigin::ExistingEntry) => out.push_str(
            "  sandbox: no --repo (the existing entry has none) — a new container is an empty workspace; rerun with --repo <url> to clone one\n",
        ),
        (None, _) => out.push_str(
            "  sandbox: no --repo (the checkout has no origin remote and none was given) — a new container is an empty workspace; rerun with --repo <url> to clone one\n",
        ),
    }
    out.push_str(&format!(
        "  --state-dir under the quecto base directory; --image {}\n",
        report.image
    ));
    let value = |value: Option<&String>| {
        value
            .map(|value| redact_url_userinfo(value))
            .unwrap_or_else(|| "(none)".to_string())
    };
    for (flag, change, current) in [
        (
            "--repo",
            &report.repository_change,
            report.repository.as_ref(),
        ),
        ("--image", &report.image_change, Some(&report.image)),
    ] {
        match change {
            None => {}
            Some(EntryValueChange::Kept) => out.push_str(&format!(
                "  kept:    {flag} {} (the existing entry's; pass {flag} to change it)\n",
                value(current)
            )),
            // The entry had no such flag: the value is now spelled out, not
            // replaced ("was (none)" read as a retag that never happened).
            Some(EntryValueChange::Rewrote { previous: None }) => out.push_str(&format!(
                "  added:   {flag} {} (the entry had none)\n",
                value(current)
            )),
            Some(EntryValueChange::Rewrote { previous }) => out.push_str(&format!(
                "  rewrote: {flag} {} (was {})\n",
                value(current),
                value(previous.as_ref())
            )),
        }
    }
    out.push_str("next:\n");
    out.push_str(&format!(
        "  1. build the image (a create never builds or pulls; the scripts drive whichever runtime `quecto container doctor` names on its runtime-cli line — run the same command with that runtime's CLI):\n     {}\n",
        report.build_command
    ));
    out.push_str("  2. quecto container doctor   — every check ✓\n");
    out.push_str(
        "  3. spawn {\"agent_id\":\"probe\",\"task\":\"run pwd\",\"container\":true} from an agent in this project; agent_cmd get_containers lists it\n",
    );
}

pub(crate) fn cmd_status(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = parse_status(args).and_then(|project| {
        let Some(build) = ctx.container_status else {
            return Err("container status not composed".to_string());
        };
        let (project, selection) = project_selection(ctx, project)?;
        Ok(build(&absolute_base_dir(ctx)?, &selection).execute(&project))
    });
    match outcome {
        Ok(status) => {
            present_status(&status, stdout);
            if status.healthy() { 0 } else { 1 }
        }
        Err(error) => {
            stderr.push_str(&error);
            if !error.ends_with('\n') {
                stderr.push('\n');
            }
            1
        }
    }
}

fn present_status(status: &StandardContainerStatus, out: &mut String) {
    out.push_str(&format!(
        "standard container at {}\n",
        status.assets_dir.display()
    ));
    let present = status.assets_present();
    let total = status.assets.len();
    let refused = status
        .assets
        .iter()
        .filter(|(_, state)| *state == AssetState::Refused)
        .count();
    if present == 0 && refused == 0 {
        out.push_str("  assets:  missing — run `quecto container init`\n");
    } else if status.assets_differing() > 0 || present < total {
        out.push_str(&format!(
            "  assets:  {present} of {total} present, {} differ from the embedded version {}\n",
            status.assets_differing(),
            status.version
        ));
        for (path, state) in &status.assets {
            let word = match state {
                AssetState::Missing => "missing",
                AssetState::Identical => "ok",
                AssetState::Differs if status.is_projects_own(path) => "yours",
                AssetState::Differs => "differs",
                AssetState::Refused => "refused",
            };
            out.push_str(&format!("           {word:<8} {}\n", path.display()));
        }
    } else {
        out.push_str(&format!(
            "  assets:  present ({present} of {total}, version {})\n",
            status.version
        ));
        for path in &status.projects_own {
            let file = path.file_name().unwrap_or_default().to_string_lossy();
            out.push_str(&format!(
                "           {file}: this project's own ({})\n",
                path.display()
            ));
        }
    }
    match &status.entry {
        Some(entry) => {
            let layer = match entry.layer {
                ContainerConfigLayer::Overlay => "overlay",
                ContainerConfigLayer::Global => "global",
            };
            let default = match status.standard_default {
                Some(StandardDefault::RepoDefault) => "default",
                Some(StandardDefault::LabelRemoved) => "default by rule",
                Some(StandardDefault::Refused) => "refused as configured",
                Some(StandardDefault::GlobalEntry { labelled: true }) => "default",
                Some(StandardDefault::GlobalEntry { labelled: false }) | None => "not default",
            };
            out.push_str(&format!(
                "  config:  {} ({default}, {layer}) in the effective configuration{}\n",
                entry.name,
                entry
                    .repository
                    .as_ref()
                    .map(|repo| format!("; --repo {repo}"))
                    .unwrap_or_else(|| "; sandbox (no --repo)".to_string())
            ));
            if status.standard_default == Some(StandardDefault::RepoDefault) {
                out.push_str(
                    "           this repo's default: `spawn container: true` selects it whatever the global file labels\n",
                );
            }
        }
        None if status.overlay_withheld => out.push_str(
            "  config:  unknown — the repo-local overlay was not applied (see trust)\n",
        ),
        None => out.push_str(
            "  config:  standard entry missing from the effective configuration — run `quecto container init`\n",
        ),
    }
    if status.overlay_withheld {
        out.push_str(
            "  trust:   withheld — the repo-local overlay is not trusted; review it and run `quecto config trust`\n",
        );
    } else {
        match &status.entry {
            Some(entry) if entry.layer == ContainerConfigLayer::Overlay => {
                out.push_str("  trust:   trusted (the repo-local overlay is applied)\n");
            }
            Some(_) => out.push_str("  trust:   n/a (the entry is declared globally)\n"),
            None => out.push_str("  trust:   n/a (no overlay entry)\n"),
        }
    }
    match (&status.image, &status.preflight_error) {
        (Some(check), _) => {
            let mark = match check.status {
                CheckStatus::Passed => "",
                CheckStatus::Warned => "! ",
                CheckStatus::Failed => "✗ ",
            };
            out.push_str(&format!("  image:   {mark}{}\n", check.detail));
            if check.status != CheckStatus::Passed && !check.remedy.is_empty() {
                out.push_str(&format!("           remedy: {}\n", check.remedy));
            }
        }
        (None, Some(reason)) => out.push_str(&format!("  image:   unknown — {reason}\n")),
        (None, None) => out.push_str("  image:   not checked (no entry to ask)\n"),
    }
    for line in &status.diagnostics {
        if status.preflight_error.as_deref() == Some(line.as_str()) {
            continue;
        }
        out.push_str(&format!("  note:    {line}\n"));
    }
    out.push_str(if status.healthy() {
        "ready: spawn {\"container\":true} from an agent in this project\n"
    } else {
        "not ready: fix the lines above, then `quecto container doctor`\n"
    });
}

#[cfg(test)]
#[path = "container_setup_e2e_tests.rs"]
mod e2e_tests;
#[cfg(test)]
#[path = "container_setup_ownership_tests.rs"]
mod ownership_tests;
#[cfg(test)]
#[path = "container_setup_tests.rs"]
mod tests;
