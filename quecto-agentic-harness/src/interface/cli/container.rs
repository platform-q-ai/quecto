//! Intentional, binary-only initialization of the standard Podman bundle.
use super::CliContext;
use crate::infrastructure::standard_assets;
use std::path::PathBuf;

pub(crate) fn cmd_container(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let Some(subcommand) = args.first().map(String::as_str) else {
        stderr.push_str("container: expected `init`\n");
        return 1;
    };
    if subcommand != "init" {
        stderr.push_str(&format!("container: unknown command `{subcommand}` (expected init)\n"));
        return 1;
    }
    let mut project: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => {
                if i + 1 >= args.len() { stderr.push_str("container init: --project requires an absolute path\n"); return 1; }
                project = Some(PathBuf::from(&args[i + 1])); i += 2;
            }
            "--dry-run" => { dry_run = true; i += 1; }
            flag => { stderr.push_str(&format!("container init: unknown option `{flag}`\n")); return 1; }
        }
    }
    let project = project.or_else(|| ctx.cwd.clone()).unwrap_or_else(|| PathBuf::from("."));
    if !project.is_absolute() {
        stderr.push_str("container init: --project must be an absolute path\n");
        return 1;
    }
    let project = match project.canonicalize() {
        Ok(path) if path.is_dir() => path,
        Ok(_) => { stderr.push_str("container init: project is not a directory\n"); return 1; }
        Err(error) => { stderr.push_str(&format!("container init: project is not accessible: {error}\n")); return 1; }
    };
    let bundle = project.join(".quecto").join("containers");
    if dry_run {
        stdout.push_str(&format!("Would materialize {} standard assets under {}\n", standard_assets::standard_assets().len(), bundle.display()));
        return 0;
    }
    match standard_assets::materialize_standard_assets_for_root(&bundle, &project) {
        Ok(created) => {
            stdout.push_str(&format!("standard container bundle ready at {}\n", bundle.display()));
            if created.is_empty() { stdout.push_str("No files changed (existing files were preserved).\n"); }
            else { stdout.push_str(&format!("Materialized {} files.\n", created.len())); }
            0
        }
        Err(error) => { stderr.push_str(&format!("container init: refused to materialize bundle: {error}\n")); 1 }
    }
}
