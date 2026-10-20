//! Workflow step references in a configuration (slice 1): a step entry
//! naming a step file, resolved within the configuration's directory.

use std::path::{Component, Path, PathBuf};

use super::ConfigError;
use crate::domain::shared::text::printable::{printable, printable_name};
use crate::infrastructure::persistence::file::json_failure as json;

pub(super) const MAX_WORKFLOW_STEP_FILE_BYTES: u64 = 64 * 1024;

const WORKFLOW_STEP_FIELDS: &[&str] = &["key", "label", "phase", "guidance"];

/// Returns `true` when resolution replaced at least one reference entry;
/// callers use that to keep the cheaper (and line/column-preserving) text
/// deserialization path for configs without references.
pub(super) fn resolve_workflow_step_entries(
    config: &mut serde_json::Value,
    config_path: &Path,
) -> Result<bool, ConfigError> {
    let Some(templates) = config
        .get_mut("workflow")
        .and_then(|workflow| workflow.get_mut("templates"))
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(false);
    };
    // Bound file loads before any reference is resolved; the engine re-checks
    // both limits post-load with full template validation.
    if templates.len() > crate::domain::workflow::MAX_TEMPLATE_COUNT {
        return Err(ConfigError::WorkflowStep(format!(
            "too many workflow templates: {} > {}",
            templates.len(),
            crate::domain::workflow::MAX_TEMPLATE_COUNT
        )));
    }
    // `Path::new("config.json").parent()` is `Some("")`, not `None`.
    let base_dir = match config_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut resolved = false;
    for template in templates {
        let id = template
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(printable_name)
            .unwrap_or_default();
        let Some(steps) = template
            .get_mut("steps")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        if steps.len() > crate::domain::workflow::MAX_STEPS_PER_TEMPLATE {
            return Err(ConfigError::WorkflowStep(format!(
                "template '{id}' has too many steps: {} > {}",
                steps.len(),
                crate::domain::workflow::MAX_STEPS_PER_TEMPLATE
            )));
        }
        for entry in steps {
            resolved |= is_workflow_step_reference(entry);
            *entry = resolve_workflow_step_entry(entry.take(), base_dir)?;
        }
    }
    Ok(resolved)
}

/// A reference object is one carrying `ref` whose other keys are all step
/// fields — including when every field is overridden, so the referenced file
/// is still loaded and validated. The one carve-out: an object that is
/// already a complete inline step (`key` + `label` + `phase`) AND carries
/// keys outside the step fields stays inline, so pre-existing configs using
/// `ref` as free-form step metadata keep working.
fn is_workflow_step_reference_object(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    object.contains_key("ref")
        && (object
            .keys()
            .all(|key| key == "ref" || WORKFLOW_STEP_FIELDS.contains(&key.as_str()))
            || !(object.contains_key("key")
                && object.contains_key("label")
                && object.contains_key("phase")))
}

fn is_workflow_step_reference(entry: &serde_json::Value) -> bool {
    match entry {
        serde_json::Value::String(_) => true,
        serde_json::Value::Object(object) => is_workflow_step_reference_object(object),
        _ => false,
    }
}

pub(super) fn resolve_workflow_step_entry(
    entry: serde_json::Value,
    base_dir: &Path,
) -> Result<serde_json::Value, ConfigError> {
    match entry {
        serde_json::Value::String(reference) => load_workflow_step(base_dir, &reference),
        serde_json::Value::Object(mut object) if is_workflow_step_reference_object(&object) => {
            let reference = object
                .remove("ref")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| ConfigError::WorkflowStep("`ref` must be a string".into()))?;
            reject_unknown_keys(&object, WORKFLOW_STEP_FIELDS, "step reference")?;
            let mut step = load_workflow_step(base_dir, &reference)?;
            let target = step.as_object_mut().expect("loaded step is an object");
            target.extend(object);
            json::from_value::<crate::domain::workflow::WorkflowTemplateStep>(step.clone())
                .map_err(|why| {
                    ConfigError::WorkflowStep(format!("{}: {why}", printable(&reference)))
                })?;
            Ok(step)
        }
        serde_json::Value::Object(object) => Ok(serde_json::Value::Object(object)),
        _ => Err(ConfigError::WorkflowStep(
            "step entry must be a string reference, reference object, or step object".into(),
        )),
    }
}

pub(super) fn load_workflow_step(
    base_dir: &Path,
    reference: &str,
) -> Result<serde_json::Value, ConfigError> {
    let mut relative = PathBuf::from(reference);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ConfigError::WorkflowStep(format!(
            "step reference must remain within the config directory: {}",
            printable(reference)
        )));
    }
    if relative.extension().is_none() {
        relative.set_extension("json");
    }
    let path = base_dir.join(relative);
    // The path holds the configuration's own reference: shown escaped.
    let shown = printable(&path.display().to_string());
    let canonical_base = base_dir
        .canonicalize()
        .map_err(|error| ConfigError::WorkflowStep(format!("{}: {error}", base_dir.display())))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|error| ConfigError::WorkflowStep(format!("{shown}: {error}")))?;
    if !canonical_path.starts_with(&canonical_base) {
        return Err(ConfigError::WorkflowStep(format!(
            "{shown}: step reference escapes config directory"
        )));
    }
    let size = canonical_path
        .metadata()
        .map_err(|error| ConfigError::WorkflowStep(format!("{shown}: {error}")))?
        .len();
    if size > MAX_WORKFLOW_STEP_FILE_BYTES {
        return Err(ConfigError::WorkflowStep(format!(
            "{shown}: step file is too large: {size} > {MAX_WORKFLOW_STEP_FILE_BYTES}"
        )));
    }
    let content = std::fs::read_to_string(&canonical_path)
        .map_err(|error| ConfigError::WorkflowStep(format!("{shown}: {error}")))?;
    let value: serde_json::Value = json::from_str(&content)
        .map_err(|why| ConfigError::WorkflowStep(format!("{shown}: {why}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ConfigError::WorkflowStep(format!("{shown}: expected a step object")))?;
    if object.contains_key("ref") {
        return Err(ConfigError::WorkflowStep(format!(
            "{shown}: expected a step object; recursive references are not allowed"
        )));
    }
    reject_unknown_keys(object, WORKFLOW_STEP_FIELDS, &shown)?;
    json::from_value::<crate::domain::workflow::WorkflowTemplateStep>(value.clone())
        .map_err(|why| ConfigError::WorkflowStep(format!("{shown}: {why}")))?;
    Ok(value)
}

fn reject_unknown_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    context: &str,
) -> Result<(), ConfigError> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ConfigError::WorkflowStep(format!(
            "{context}: unknown field `{}`",
            printable_name(key)
        )));
    }
    Ok(())
}
