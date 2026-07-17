//! Workflow template directory discovery (workflow-composable-templates PRD
//! §3.2, slice 2): the `workflow.dir` precedence chain and the loader that
//! builds a template library from the top-level `*.json` files of a workflow
//! directory. Split out of `config.rs`; it shares that module's private
//! step-reference resolver so directory templates and inline configs load
//! through identical slice-1 rules.

use std::path::{Path, PathBuf};

use super::{Config, ConfigError, resolve_workflow_step_entry};

/// Outcome of workflow template discovery (PRD workflow-composable-templates
/// §3.2, slice 2).
#[derive(Debug, Clone)]
pub struct WorkflowTemplateDiscovery {
    /// The resolved template library the engine should run with.
    pub templates: Vec<crate::domain::workflow::WorkflowTemplate>,
    /// The directory templates were loaded from; `None` means the inline
    /// `workflow.templates` fallback was used.
    pub source_dir: Option<PathBuf>,
    /// Startup warning to surface to the user (e.g. a discovered directory
    /// shadowing inline `workflow.templates`).
    pub warning: Option<String>,
}

/// Discover the workflow template library for a session (slice 2).
///
/// Precedence: `workflow.dir` (resolved against `cwd` when relative) →
/// `<cwd>/.quecto/workflows` if it exists → `<home>/.quecto/workflows` if it
/// exists → the inline `workflow.templates` fallback. A directory always wins
/// over inline templates; when both are present the inline templates are
/// ignored with a startup warning.
pub fn discover_workflow_templates(
    config: &Config,
    cwd: &Path,
    home_dir: Option<&Path>,
) -> Result<WorkflowTemplateDiscovery, ConfigError> {
    // `auto_discovered` distinguishes an explicitly configured `workflow.dir`
    // (a deliberate user choice) from a directory picked up implicitly from the
    // repo-local/home precedence chain. The implicit case silently switches the
    // session away from the built-in default templates, so it is surfaced with
    // a startup warning below.
    let (source_dir, auto_discovered) = if let Some(configured) = config.workflow.dir.as_deref() {
        // An explicitly configured workflow.dir that does not exist is a hard
        // error — never a silent fall-through to another source.
        let dir = cwd.join(configured);
        if !dir.is_dir() {
            return Err(ConfigError::WorkflowTemplate(format!(
                "workflow.dir is not a directory: {}",
                dir.display()
            )));
        }
        (Some(dir), false)
    } else {
        let discovered = [Some(cwd), home_dir]
            .into_iter()
            .flatten()
            .map(|root| root.join(".quecto/workflows"))
            .find(|dir| dir.is_dir());
        (discovered, true)
    };
    let Some(dir) = source_dir else {
        // Inline `workflow.templates` fallback (AC5); an empty library keeps
        // the engine's built-in defaults working out of the box.
        return Ok(WorkflowTemplateDiscovery {
            templates: config.workflow.templates.clone(),
            source_dir: None,
            warning: None,
        });
    };
    let templates = load_workflow_templates_from_dir(&dir)?;
    // A resolved workflow directory is the single source of truth for the
    // session; zero templates is a hard error, never a silent fall-through to
    // the engine's built-in defaults (which would let a misconfigured or
    // template-less directory quietly run the shipped feature/refactor
    // workflows the user believed they had replaced).
    if templates.is_empty() {
        return Err(ConfigError::WorkflowTemplate(format!(
            "workflow directory {} contains no templates (no top-level *.json files)",
            dir.display()
        )));
    }
    let warning = if !config.workflow.templates.is_empty() {
        // A directory always wins over inline templates; say so loudly so the
        // ignored inline library is never a silent surprise.
        Some(format!(
            "workflow template directory {} is in use; inline workflow.templates are ignored",
            dir.display()
        ))
    } else if auto_discovered {
        // No inline templates, but a directory was picked up implicitly from
        // the precedence chain: surface that the session runs those templates
        // instead of the built-in defaults, so the switch is never invisible.
        Some(format!(
            "workflow templates loaded from discovered directory {}; built-in default templates are not in use",
            dir.display()
        ))
    } else {
        None
    };
    Ok(WorkflowTemplateDiscovery {
        templates,
        source_dir: Some(dir),
        warning,
    })
}

/// Template-file fields (strict, PRD Decision 2). `id` is intentionally
/// absent: the template id IS the filename stem, so an explicit `id` field is
/// a load error (two sources of truth could disagree).
const WORKFLOW_TEMPLATE_FIELDS: &[&str] =
    &["label", "description", "when_to_use", "steps", "guards"];
const WORKFLOW_TEMPLATE_STEP_FIELDS: &[&str] = &["key", "label", "phase", "guidance"];
const WORKFLOW_TEMPLATE_REFERENCE_FIELDS: &[&str] = &["ref", "key", "label", "phase", "guidance"];
const WORKFLOW_TEMPLATE_GUARD_FIELDS: &[&str] = &["commands", "before_step_key", "message"];

/// Maximum raw size of one directory template file (256 KiB). Combined with
/// `MAX_TEMPLATE_COUNT`, this bounds repository-controlled startup input before
/// JSON parsing and matches the existing by-value workflow-spec ceiling.
pub(crate) const MAX_WORKFLOW_TEMPLATE_FILE_BYTES: u64 =
    crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES as u64;

fn reject_unknown_template_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    context: &str,
) -> Result<(), ConfigError> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ConfigError::WorkflowTemplate(format!(
            "{context}: unknown field `{key}`"
        )));
    }
    Ok(())
}

/// Load every workflow template from the top-level `*.json` files of `dir`
/// (template id = filename stem; `steps/` and all subfolders are never scanned
/// for templates). Fails fast naming the offending file on any load error.
pub fn load_workflow_templates_from_dir(
    dir: &Path,
) -> Result<Vec<crate::domain::workflow::WorkflowTemplate>, ConfigError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| ConfigError::WorkflowTemplate(format!("{}: {error}", dir.display())))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    if files.len() > crate::domain::workflow::MAX_TEMPLATE_COUNT {
        return Err(ConfigError::WorkflowTemplate(format!(
            "{}: too many workflow templates: {} > {}",
            dir.display(),
            files.len(),
            crate::domain::workflow::MAX_TEMPLATE_COUNT
        )));
    }
    files
        .iter()
        .map(|path| load_workflow_template_file(dir, path))
        .collect()
}

fn load_workflow_template_file(
    dir: &Path,
    path: &Path,
) -> Result<crate::domain::workflow::WorkflowTemplate, ConfigError> {
    let context = path.display().to_string();
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| {
            ConfigError::WorkflowTemplate(format!("{context}: template filename must be UTF-8"))
        })?
        .to_owned();
    let size = path
        .metadata()
        .map_err(|error| ConfigError::WorkflowTemplate(format!("{context}: {error}")))?
        .len();
    if size > MAX_WORKFLOW_TEMPLATE_FILE_BYTES {
        return Err(ConfigError::WorkflowTemplate(format!(
            "{context}: template file is too large: {size} > {MAX_WORKFLOW_TEMPLATE_FILE_BYTES}"
        )));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| ConfigError::WorkflowTemplate(format!("{context}: {error}")))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|error| ConfigError::WorkflowTemplate(format!("{context}: {error}")))?;
    let serde_json::Value::Object(mut object) = value else {
        return Err(ConfigError::WorkflowTemplate(format!(
            "{context}: expected a template object"
        )));
    };
    // Strict parsing (PRD Decision 2): typos and an explicit `id` field are
    // both load errors naming the file.
    reject_unknown_template_keys(&object, WORKFLOW_TEMPLATE_FIELDS, &context)?;
    {
        let steps = object
            .get_mut("steps")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| {
                ConfigError::WorkflowTemplate(format!(
                    "{context}: template must have a steps array"
                ))
            })?;
        if steps.is_empty() {
            return Err(ConfigError::WorkflowTemplate(format!(
                "{context}: template must define at least one step"
            )));
        }
        if steps.len() > crate::domain::workflow::MAX_STEPS_PER_TEMPLATE {
            return Err(ConfigError::WorkflowTemplate(format!(
                "{context}: too many steps: {} > {}",
                steps.len(),
                crate::domain::workflow::MAX_STEPS_PER_TEMPLATE
            )));
        }
        // Directory templates are a strict file format: validate nested inline
        // steps here rather than changing the shared domain structs, whose
        // lenient deserialization is retained for backward-compatible inline
        // configs and forward-compatible by-value WorkflowSpec payloads.
        for entry in steps.iter() {
            if let serde_json::Value::Object(step) = entry {
                let allowed = if step.contains_key("ref") {
                    WORKFLOW_TEMPLATE_REFERENCE_FIELDS
                } else {
                    WORKFLOW_TEMPLATE_STEP_FIELDS
                };
                reject_unknown_template_keys(step, allowed, &format!("{context}: step entry"))?;
            }
        }
        // Step references resolve relative to the workflow directory, through
        // the same slice-1 resolver inline configs use (bounds, no recursion,
        // strict step fields, containment within the directory).
        for entry in steps {
            *entry = resolve_workflow_step_entry(entry.take(), dir)
                .map_err(|error| ConfigError::WorkflowTemplate(format!("{context}: {error}")))?;
        }
    }
    if let Some(guards) = object.get("guards").and_then(serde_json::Value::as_array) {
        for guard in guards {
            if let serde_json::Value::Object(guard) = guard {
                reject_unknown_template_keys(
                    guard,
                    WORKFLOW_TEMPLATE_GUARD_FIELDS,
                    &format!("{context}: guard"),
                )?;
            }
        }
    }
    object.insert("id".into(), serde_json::Value::String(id));
    let template: crate::domain::workflow::WorkflowTemplate =
        serde_json::from_value(serde_json::Value::Object(object))
            .map_err(|error| ConfigError::WorkflowTemplate(format!("{context}: {error}")))?;
    let mut seen = std::collections::HashSet::new();
    if let Some(step) = template.steps.iter().find(|step| !seen.insert(&step.key)) {
        return Err(ConfigError::WorkflowTemplate(format!(
            "{context}: duplicate step key `{}`",
            step.key
        )));
    }
    Ok(template)
}

#[cfg(test)]
#[path = "config_discovery_tests.rs"]
mod tests;
