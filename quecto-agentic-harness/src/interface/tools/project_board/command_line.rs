//! A board request from a command line: `op` (`link-pr` or `link_pr`) and
//! its `--field value` flags, each value read as its field's type, so the
//! command line reaches [`request::parse`](super::request::parse) as the
//! tool's JSON does.
use serde_json::{Map, Value};

use super::json::strict;
use super::request::{Kind, RequestError, fields_of};
use crate::domain::project_board::value_objects::telemetry::FaultKind;

/// The request `op` and `flags` name. A list of ids is comma-separated (an
/// empty value is the empty list); `--repeated`, `--items`, `--team`,
/// `--done-when` and `--links`
/// are JSON (`--team null` takes a draft's team away); `--parent ""`
/// takes a draft's parent away; `--expected-accepted ""` (or `null`) says
/// the task named no accepted plan.
///
/// # Errors
/// An unknown op, a flag the op does not take or gives twice, a flag
/// without a value, an empty id in a list, or a JSON flag that is not
/// JSON.
pub fn request(op: &str, flags: &[String]) -> Result<Value, RequestError> {
    let refused = |field: &str, problem: &str, fault| RequestError::of(field, problem, fault);
    let op = op.replace('-', "_");
    // A name that is no op is never echoed back.
    let fields = fields_of(&op)
        .ok_or_else(|| refused("op", "is not an op of the board", FaultKind::InvalidValue))?;
    let mut object = Map::new();
    object.insert("op".into(), Value::String(op.clone()));
    let mut rest = flags.iter();
    while let Some(flag) = rest.next() {
        let name = flag
            .strip_prefix("--")
            .map(|name| name.replace('-', "_"))
            .ok_or_else(|| refused("flags", "expected --<field> <value>", FaultKind::WrongType))?;
        let field = fields
            .iter()
            .find(|field| field.name == name)
            // A field the op does not take is never echoed back as one.
            .ok_or_else(|| RequestError::stray("flags", format!("{op} takes no such field"), 1))?;
        let text = rest
            .next()
            .ok_or_else(|| refused(&name, "needs a value", FaultKind::Missing))?;
        let value = match field.kind {
            Kind::Boolean => match text.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => Value::String(text.clone()),
            },
            Kind::Slugs if text.is_empty() => Value::Array(vec![]),
            Kind::Slugs => {
                let ids: Vec<&str> = text.split(',').collect();
                if ids.iter().any(|id| id.is_empty()) {
                    return Err(refused(&name, "names an empty id", FaultKind::InvalidValue));
                }
                Value::Array(ids.into_iter().map(|id| Value::String(id.into())).collect())
            }
            Kind::OptionalSlug if text.is_empty() => Value::Null,
            Kind::AcceptedVersion if text.is_empty() || text == "null" => Value::Null,
            Kind::Pr
            | Kind::Index
            | Kind::Offset
            | Kind::Version
            | Kind::AcceptedVersion
            | Kind::Run => text
                .parse::<u64>()
                .map(Value::from)
                .unwrap_or_else(|_| Value::String(text.clone())),
            Kind::Slug
            | Kind::OptionalSlug
            | Kind::Text
            | Kind::Timestamp
            | Kind::Rev
            | Kind::Status
            | Kind::NewStatus
            | Kind::TaskKind
            | Kind::RunEnd
            | Kind::RunStop
            | Kind::Path
            | Kind::ObjectId => Value::String(text.clone()),
            Kind::Repeated | Kind::PlanItems | Kind::Team | Kind::Lines => {
                strict(text).map_err(|error| {
                    refused(&name, &format!("must be JSON: {error}"), error.fault())
                })?
            }
        };
        if object.insert(name.clone(), value).is_some() {
            return Err(refused(&name, "is given twice", FaultKind::DuplicateKey));
        }
    }
    Ok(Value::Object(object))
}
