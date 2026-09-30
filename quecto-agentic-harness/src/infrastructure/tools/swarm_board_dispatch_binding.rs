//! Binding a board call's arguments to its method's signature (#2270), as
//! Python binds a call, and what a refusal raised while binding them found
//! wrong, for the call's records (#2341).
//!
//! The records name arguments by the `swarm` tool's schema only
//! ([`BOARD_OPS`]): every name recorded is a [`SchemaField`], the table's
//! own `&'static str`, which only [`schema_field`] makes, never the
//! member's text. An unexpected key
//! is counted and named only when some op's schema has that field; a
//! parameter no schema field names (a harness-internal method's) is left
//! out; and `arguments` stands for the arguments as a whole (text that is
//! no JSON, or a value the board cannot hold under a key no schema field
//! names).
use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::super::swarm_board_ops::{BOARD_OPS, op_spec};
use super::method::{Method, Parameter};
use crate::domain::swarm::{ArgumentFaults, BoardError, RefusalKind, UnexpectedArgs, WrongTypeArg};

/// The name recorded for the arguments as a whole.
pub(super) const WHOLE: &str = "arguments";

/// Binds `args` to `parameters` as Python binds a call: positionally from
/// an array, by name from an object, then each unbound parameter's default.
pub(super) fn bind(
    method: Method,
    parameters: &[Parameter],
    args: Value,
) -> Result<Vec<Value>, BoardError> {
    let name = method.name();
    let mut slots: Vec<Option<Value>> = vec![None; parameters.len()];
    match args {
        Value::Array(values) => {
            if values.len() > parameters.len() {
                return Err(BoardError::new(
                    RefusalKind::Calling,
                    format!(
                        "{name}: takes {} arguments, {} given",
                        parameters.len(),
                        values.len()
                    ),
                ));
            }
            for (slot, value) in slots.iter_mut().zip(values) {
                *slot = Some(value);
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                let Some(index) = parameters.iter().position(|p| p.name == key) else {
                    return Err(BoardError::new(
                        RefusalKind::Calling,
                        format!("{name}: unexpected argument {key}"),
                    ));
                };
                slots[index] = Some(value);
            }
        }
        _ => {
            return Err(BoardError::new(
                RefusalKind::Calling,
                format!("{name}: arguments must be a JSON array or object"),
            ));
        }
    }
    slots
        .into_iter()
        .zip(parameters)
        .map(|(slot, parameter)| match (slot, parameter.default) {
            (Some(value), _) => Ok(value),
            (None, Some(default)) => Ok(default()),
            (None, None) => Err(BoardError::new(
                RefusalKind::Calling,
                format!("{name}: missing required argument {}", parameter.name),
            )),
        })
        .collect()
}

/// A field name of some board op's schema (#2346 review M): the
/// [`BOARD_OPS`] table's own `&'static str`. Its field is private and only
/// [`schema_field`] makes one, so no text from outside (a member's key, a
/// caller's string) can become a name a record carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaField(&'static str);

impl SchemaField {
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// The field `key` names in some board op's schema: the table's own name,
/// so a record names it and never the text it was given as; `None` for
/// any other key.
pub fn schema_field(key: &str) -> Option<SchemaField> {
    BOARD_OPS
        .iter()
        .flat_map(|spec| spec.args.iter())
        .map(|arg| arg.name)
        .find(|name| *name == key)
        .map(SchemaField)
}

/// A name an unreadable argument is recorded under: its schema field, or
/// [`WHOLE`] for the arguments as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArgName {
    Field(SchemaField),
    Whole,
}

impl ArgName {
    fn as_str(self) -> &'static str {
        match self {
            Self::Field(field) => field.as_str(),
            Self::Whole => WHOLE,
        }
    }
}

/// What a refusal raised while binding a call's arguments found wrong
/// (#2341), as the dispatcher records it. Every name is a [`SchemaField`]
/// (or [`WHOLE`]) and every type label the schema table's own, and its
/// fields are private: only [`faults`] and [`unreadable_arguments`] make
/// one with anything in it, so the allowlist holds by type, in a release
/// build too (#2346 review M). [`Self::record`] is the event log's form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BindingFaults {
    missing: Vec<SchemaField>,
    unexpected: u64,
    unexpected_known: Vec<SchemaField>,
    wrong_type: Vec<(SchemaField, &'static str)>,
    unreadable: Vec<ArgName>,
}

impl BindingFaults {
    /// Nothing found wrong.
    pub const NONE: Self = Self {
        missing: Vec::new(),
        unexpected: 0,
        unexpected_known: Vec::new(),
        wrong_type: Vec::new(),
        unreadable: Vec::new(),
    };

    pub fn is_empty(&self) -> bool {
        self == &Self::NONE
    }

    /// The faults as the `swarm_op` record carries them.
    pub fn record(&self) -> ArgumentFaults {
        let names = |fields: &[SchemaField]| {
            fields
                .iter()
                .map(|field| field.as_str().to_owned())
                .collect()
        };
        let recorded = ArgumentFaults {
            missing_args: names(&self.missing),
            unexpected_args: (self.unexpected > 0).then(|| UnexpectedArgs {
                count: self.unexpected,
                known: names(&self.unexpected_known),
            }),
            wrong_type_args: self
                .wrong_type
                .iter()
                .map(|(field, expected)| WrongTypeArg {
                    arg: field.as_str().to_owned(),
                    expected: (*expected).to_owned(),
                })
                .collect(),
            unreadable_args: self
                .unreadable
                .iter()
                .map(|name| name.as_str().to_owned())
                .collect(),
        };
        debug_assert!(
            allowlisted(&recorded),
            "argument faults name schema fields only"
        );
        recorded
    }
}

/// Whether every name `faults` holds is a schema field (or [`WHOLE`]):
/// the allowlist every record is held to, which [`BindingFaults`] keeps by
/// type; checked again, in debug builds, on every record made.
pub(super) fn allowlisted(faults: &ArgumentFaults) -> bool {
    let known = |name: &String| name == WHOLE || schema_field(name).is_some();
    let unexpected = faults
        .unexpected_args
        .iter()
        .flat_map(|unexpected| unexpected.known.iter());
    let wrong = faults.wrong_type_args.iter().map(|wrong| &wrong.arg);
    faults
        .missing_args
        .iter()
        .chain(unexpected)
        .chain(wrong)
        .chain(faults.unreadable_args.iter())
        .all(known)
        && faults.names_are_kinds()
}

/// The faults a call's records keep: `faults` for a `calling` refusal (the
/// binding's own: an argument missing or unexpected) or an `invalid` one
/// (a value refused: before the board, or by the board's own checks,
/// whose cause the faults need not be; see [`faults`]), none for any
/// other outcome.
pub(super) fn recorded<T>(
    outcome: &Result<T, RefusalKind>,
    faults: Option<BindingFaults>,
) -> BindingFaults {
    match outcome {
        Err(RefusalKind::Calling | RefusalKind::Invalid) => faults.unwrap_or_default(),
        _ => BindingFaults::NONE,
    }
}

/// What `args` gets wrong against `method`'s signature: the required
/// parameters not given, the keys (or positions) it has no parameter for,
/// and, for a member-facing op ([`op_spec`]), the values of another JSON
/// type than its schema's (an array's items by their own type only: an
/// `evidence` item need only be an object, its properties unchecked).
///
/// A type mismatch is found against the schema, not the board's checks,
/// and is recorded for any `invalid` refusal: the board refuses one value
/// (an empty acceptance list) while another differs from the schema (a
/// request id given as a number, which the board binds untyped), and the
/// record names the one that differs, which need not be the cause.
pub(super) fn faults(method: Method, args: &Value) -> BindingFaults {
    let parameters = method.parameters();
    let typed = op_spec(method.name()).is_some();
    let mut faults = BindingFaults::NONE;
    let mut given = vec![false; parameters.len()];
    let mut bound = |faults: &mut BindingFaults, index: usize, value: &Value| {
        given[index] = true;
        let name = parameters[index].name;
        if let (true, Some(field), Some(expected)) =
            (typed, schema_field(name), expected_type(name))
            && !expected.admits(value)
        {
            faults.wrong_type.push((field, expected.label.as_str()));
        }
    };
    match args {
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                match index < parameters.len() {
                    true => bound(&mut faults, index, value),
                    false => unexpected(&mut faults, None),
                }
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                match parameters.iter().position(|p| p.name == key) {
                    Some(index) => bound(&mut faults, index, value),
                    None => unexpected(&mut faults, schema_field(key)),
                }
            }
        }
        _ => faults.unreadable.push(ArgName::Whole),
    }
    faults.missing = parameters
        .iter()
        .zip(given)
        .filter(|(parameter, given)| !given && parameter.default.is_none())
        .filter_map(|(parameter, _)| schema_field(parameter.name))
        .collect();
    faults
}

fn unexpected(faults: &mut BindingFaults, known: Option<SchemaField>) {
    faults.unexpected += 1;
    faults.unexpected_known.extend(known);
}

/// The faults of member text the board's value type cannot hold (#2341),
/// given the keys `unreadable` found (`None`: the text as a whole): each
/// key's schema field, and [`WHOLE`] once for the text as a whole or a
/// key no schema field names.
pub fn unreadable_arguments(keys: Option<Vec<String>>) -> BindingFaults {
    let keys = keys.unwrap_or_default();
    let mut names: Vec<ArgName> = Vec::new();
    let mut whole = keys.is_empty();
    for key in &keys {
        match schema_field(key).map(ArgName::Field) {
            Some(name) if !names.contains(&name) => names.push(name),
            Some(_) => {}
            None => whole = true,
        }
    }
    if whole {
        names.push(ArgName::Whole);
    }
    BindingFaults {
        unreadable: names,
        ..BindingFaults::NONE
    }
}

/// The JSON type a schema field expects: its `type` (one or several) and,
/// for an array, its items' `type`, and the label a record gives it
/// (`integer`, `null_or_string`, `array_of_string`,
/// `null_or_array_of_integer`, ...): `null` first, so a label reads one
/// way.
#[derive(Debug)]
pub(super) struct Expected {
    types: Vec<String>,
    items: Vec<String>,
    pub(super) label: String,
}

impl Expected {
    fn of(schema: &Value) -> Self {
        let types = type_names(&schema["type"]);
        let items = type_names(&schema["items"]["type"]);
        // `null` first: `null_or_array_of_integer` reads one way, where
        // `array_of_integer_or_null` could be an array of either.
        let (null, others): (Vec<&String>, Vec<&String>) =
            types.iter().partition(|name| *name == "null");
        let label = null
            .into_iter()
            .chain(others)
            .map(|name| match (name.as_str(), items.is_empty()) {
                ("array", false) => format!("array_of_{}", items.join("_or_")),
                _ => name.clone(),
            })
            .collect::<Vec<_>>()
            .join("_or_");
        Self {
            types,
            items,
            label,
        }
    }

    /// Whether `value` is of the type expected (an array's items too).
    pub(super) fn admits(&self, value: &Value) -> bool {
        let items_admitted = match value {
            Value::Array(items) => items.iter().all(|item| admitted(&self.items, item)),
            _ => true,
        };
        admitted(&self.types, value) && items_admitted
    }
}

fn type_names(schema_type: &Value) -> Vec<String> {
    match schema_type {
        Value::String(name) => vec![name.clone()],
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether `value` is one of `types` (JSON Schema's: an integer is a
/// number too); anything, when there are none.
fn admitted(types: &[String], value: &Value) -> bool {
    let found = match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    types.is_empty()
        || types
            .iter()
            .any(|name| name == found || (name == "number" && found == "integer"))
}

/// The type the schema expects of field `name` (one schema per field,
/// which `tool_schema` asserts), read from [`BOARD_OPS`] once.
pub(super) fn expected_type(name: &str) -> Option<&'static Expected> {
    static EXPECTED: OnceLock<HashMap<&'static str, Expected>> = OnceLock::new();
    EXPECTED
        .get_or_init(|| {
            BOARD_OPS
                .iter()
                .flat_map(|spec| spec.args.iter())
                .map(|arg| {
                    let schema: Value =
                        serde_json::from_str(arg.json_type).expect("a checked-in JSON schema");
                    (arg.name, Expected::of(&schema))
                })
                .collect()
        })
        .get(name)
}

#[cfg(test)]
#[path = "swarm_board_dispatch_binding_tests.rs"]
mod tests;
