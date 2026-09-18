//! `quecto container init` and `quecto container status` (#2024 S4e):
//! parse the arguments, invoke the composed use case for the project
//! (the working directory unless `--project` names another), present
//! the report. `init` prints the files it materialised, the entry it
//! wrote and where, and the exact `podman build` command that is the one
//! step left; `status` prints one line each for the assets, the entry,
//! the trust and the image (asked of the entry's own create preflight)
//! and exits 1 while anything is missing. The interface resolves no
//! config, reads no file and runs no script itself.

use std::path::{Path, PathBuf};

use super::CliContext;
use crate::application::configuration::dto::ConfigSelection;
use crate::application::environments::dto::{
    AssetState, CheckStatus, ContainerConfigLayer, RepositoryOrigin, StandardContainerReport,
    StandardContainerRequest, StandardContainerStatus,
};
use crate::domain::redaction::redact_url_userinfo;

const INIT_USAGE: &str = "usage: quecto container init [--project <abs dir>] [--repo <url>] [--image <tag>] [--dry-run]\n";
const STATUS_USAGE: &str = "usage: quecto container status [--project <abs dir>]\n";

struct InitArgs {
    project: Option<PathBuf>,
    repository: Option<String>,
    image: Option<String>,
    dry_run: bool,
}

fn parse_init(args: &[String]) -> Result<InitArgs, String> {
    let mut parsed = InitArgs {
        project: None,
        repository: None,
        image: None,
        dry_run: false,
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
            "--project" => parsed.project = Some(PathBuf::from(value("--project", &mut rest)?)),
            "--repo" => parsed.repository = Some(value("--repo", &mut rest)?),
            "--image" => parsed.image = Some(value("--image", &mut rest)?),
            "--dry-run" => parsed.dry_run = true,
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
                project = Some(PathBuf::from(
                    rest.next()
                        .filter(|v| !v.is_empty())
                        .ok_or_else(|| format!("--project requires a value\n{STATUS_USAGE}"))?,
                ));
            }
            other => return Err(format!("unknown argument {other}\n{STATUS_USAGE}")),
        }
    }
    Ok(project)
}

/// The project the command works on, and the configuration selection its
/// overlay belongs to: `--project` (absolute) or the working directory.
/// A `--project` elsewhere than the working directory selects THAT
/// directory's layers, so the overlay written is the project's.
fn project_selection(
    ctx: &CliContext,
    project: Option<PathBuf>,
) -> Result<(PathBuf, ConfigSelection), String> {
    let project = match project {
        Some(path) => {
            if !path.is_absolute() {
                return Err(format!(
                    "--project must be an absolute path: {}",
                    path.display()
                ));
            }
            path
        }
        None => ctx
            .cwd
            .clone()
            .ok_or_else(|| "the working directory is unknown; pass --project".to_string())?,
    };
    let project = std::fs::canonicalize(&project)
        .map_err(|error| format!("project {} is not accessible: {error}", project.display()))?;
    let selection = if ctx.cwd.as_deref() == Some(project.as_path()) {
        ctx.config_selection()?
    } else {
        let mut for_project = ctx.clone();
        for_project.cwd = Some(project.clone());
        for_project.config_selection()?
    };
    Ok((project, selection))
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
        build(&ctx.base_dir(), &selection)
            .execute(&StandardContainerRequest {
                project,
                repository: parsed.repository,
                image: parsed.image,
                dry_run: parsed.dry_run,
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

/// The build command init prints: the image the entry launches, built
/// from the materialised Containerfile with the bundle as context.
pub fn build_command(image: &str, assets_dir: &Path) -> String {
    format!(
        "podman build -t {image} -f {} {}",
        assets_dir.join("Containerfile").display(),
        assets_dir.display()
    )
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
    for path in &report.differing {
        out.push_str(&format!(
            "  kept   {} (differs from the embedded version {}; delete it to refresh)\n",
            path.display(),
            report.version
        ));
    }
    if report.written.is_empty() && !report.dry_run {
        out.push_str("  no files changed (existing files are never replaced)\n");
    }
    let entry = &report.entry;
    let location = entry
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "the repo-local overlay".to_string());
    out.push_str(&format!(
        "container config \"{}\" {} container_configs.{} in {location} (trusted for exactly these bytes)\n",
        entry.name,
        if report.dry_run { "would be written as" } else { "written as" },
        entry.name
    ));
    match &entry.existing_default {
        None => out.push_str("  default: true — `spawn container: true` selects it\n"),
        Some(other) => out.push_str(&format!(
            "  not the default: {other} is already the default; select it by name (container_config: \"{}\") or move the label with `quecto config set`\n",
            entry.name
        )),
    }
    match (&report.repository, &report.repository_origin) {
        (Some(url), RepositoryOrigin::Explicit) => out.push_str(&format!(
            "  --repo {} (as given): a new container is a fresh clone of it\n",
            redact_url_userinfo(url)
        )),
        (Some(url), _) => out.push_str(&format!(
            "  --repo {} (the checkout's origin remote): a new container is a fresh clone of it\n",
            redact_url_userinfo(url)
        )),
        (None, _) => out.push_str(
            "  sandbox: no --repo (the checkout has no origin remote and none was given) — a new container is an empty workspace; rerun with --repo <url> to clone one\n",
        ),
    }
    out.push_str(&format!(
        "  --state-dir under the quecto base directory; --image {}\n",
        report.image
    ));
    out.push_str("next:\n");
    out.push_str(&format!(
        "  1. build the image (a create never builds or pulls):\n     {}\n",
        build_command(&report.image, &report.assets_dir)
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
        Ok(build(&ctx.base_dir(), &selection).execute(&project))
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
    if present == 0 {
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
                AssetState::Differs => "differs",
            };
            out.push_str(&format!("           {word:<8} {}\n", path.display()));
        }
    } else {
        out.push_str(&format!(
            "  assets:  present ({present} of {total}, version {})\n",
            status.version
        ));
    }
    match &status.entry {
        Some(entry) => {
            let layer = match entry.layer {
                ContainerConfigLayer::Overlay => "overlay",
                ContainerConfigLayer::Global => "global",
            };
            out.push_str(&format!(
                "  config:  {} ({}, {layer}) in the effective configuration{}\n",
                entry.name,
                if entry.default { "default" } else { "not default" },
                entry
                    .repository
                    .as_ref()
                    .map(|repo| format!("; --repo {repo}"))
                    .unwrap_or_else(|| "; sandbox (no --repo)".to_string())
            ));
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
#[path = "container_setup_tests.rs"]
mod tests;
