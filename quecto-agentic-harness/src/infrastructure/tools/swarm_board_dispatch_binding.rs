//! Binding a board call's arguments to its method's signature (#2270), as
//! Python binds a call, and what a refusal raised while binding them found
//! wrong, for the call's records (#2341); and the task-item names a
//! structured op and its task id are also taken by (#2478,
//! [`task_item_names`]).
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

use self::task_item_names::ID_ALIAS;
use super::super::swarm_board_ops::{BOARD_OPS, op_spec};
use super::method::{Method, Parameter};
use crate::domain::swarm::{ArgumentFaults, BoardError, RefusalKind, UnexpectedArgs, WrongTypeArg};

/// The name recorded for the arguments as a whole.
pub(super) const WHOLE: &str = "arguments";

/// Binds `args` to `parameters` as Python binds a call: positionally from
/// an array, by name from an object, then each unbound parameter's default;
/// or the refusal ([`refusal`]). The dispatcher calls the two halves
/// itself, reading a refusal's faults between them.
#[cfg(test)]
pub(super) fn bind(
    method: Method,
    parameters: &[Parameter],
    args: Value,
) -> Result<Vec<Value>, BoardError> {
    match refusal(method, parameters, &args) {
        Some(refusal) => Err(refusal),
        None => Ok(slotted(parameters, args)),
    }
}

/// Why `args` does not bind to `parameters`, as Python refuses the call,
/// read without taking the arguments (#2346 final review: a refusal's
/// faults are read from them only then); `None` when they bind.
pub(super) fn refusal(
    method: Method,
    parameters: &[Parameter],
    args: &Value,
) -> Option<BoardError> {
    let name = method.name();
    let given: Vec<bool> = match args {
        Value::Array(values) if values.len() > parameters.len() => {
            return Some(BoardError::new(
                RefusalKind::Calling,
                format!(
                    "{name}: takes {} arguments, {} given",
                    parameters.len(),
                    values.len()
                ),
            ));
        }
        Value::Array(values) => (0..parameters.len())
            .map(|index| index < values.len())
            .collect(),
        Value::Object(fields) => {
            let position = |key: &str| parameters.iter().position(|p| p.name == key);
            if let Some(key) = fields.keys().find(|key| position(key).is_none()) {
                return Some(BoardError::new(
                    RefusalKind::Calling,
                    format!("{name}: unexpected argument {key}"),
                ));
            }
            parameters
                .iter()
                .map(|parameter| fields.contains_key(parameter.name))
                .collect()
        }
        _ => {
            return Some(BoardError::new(
                RefusalKind::Calling,
                format!("{name}: arguments must be a JSON array or object"),
            ));
        }
    };
    parameters
        .iter()
        .zip(given)
        .find(|(parameter, given)| !given && parameter.default.is_none())
        .map(|(parameter, _)| {
            BoardError::new(
                RefusalKind::Calling,
                format!("{name}: missing required argument {}", parameter.name),
            )
        })
}

/// `args`, which [`refusal`] found to bind, bound to `parameters`.
pub(super) fn slotted(parameters: &[Parameter], args: Value) -> Vec<Value> {
    let mut slots: Vec<Option<Value>> = vec![None; parameters.len()];
    match args {
        Value::Array(values) => {
            for (slot, value) in slots.iter_mut().zip(values) {
                *slot = Some(value);
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                if let Some(index) = parameters.iter().position(|p| p.name == key) {
                    slots[index] = Some(value);
                }
            }
        }
        _ => {}
    }
    debug_assert!(
        slots
            .iter()
            .zip(parameters)
            .all(|(slot, parameter)| slot.is_some() || parameter.default.is_some()),
        "only arguments that bind are slotted"
    );
    slots
        .into_iter()
        .zip(parameters)
        .map(|(slot, parameter)| {
            slot.or_else(|| parameter.default.map(|default| default()))
                .unwrap_or(Value::Null)
        })
        .collect()
}

/// A field name of some board op's schema (#2346 review M): the
/// [`BOARD_OPS`] table's own `&'static str`. Its field is private and only
/// [`schema_field`] makes one, so no text from outside (a member's key, a
/// caller's string) can become a name a record carries.
///
/// ```
/// use quecto::infrastructure::tools::swarm_board_dispatch::{SchemaField, schema_field};
/// let field: SchemaField = schema_field("task_id").expect("a schema field");
/// assert_eq!(field.as_str(), "task_id");
/// assert!(schema_field("sk-ant-api03-anything").is_none());
/// ```
///
/// No text builds one: its field is private,
///
/// ```compile_fail
/// use quecto::infrastructure::tools::swarm_board_dispatch::SchemaField;
/// let field = SchemaField("sk-ant-api03-anything");
/// ```
///
/// and it has no conversion from a string, nor a deserializer.
///
/// ```compile_fail
/// use quecto::infrastructure::tools::swarm_board_dispatch::SchemaField;
/// let field = SchemaField::from(String::from("sk-ant-api03-anything"));
/// ```
///
/// ```compile_fail
/// use quecto::infrastructure::tools::swarm_board_dispatch::SchemaField;
/// let field: SchemaField = serde_json::from_str(r#""sk-ant-api03-anything""#).unwrap();
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaField(&'static str);

impl SchemaField {
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// The field `key` names in the `swarm` tool's schema: some board op's
/// (the table's own name), or `task_item_id`, the schema's task-item name
/// for `task_id` (#2478, [`ID_ALIAS`]), so a record names it and never the
/// text it was given as; `None` for any other key.
pub fn schema_field(key: &str) -> Option<SchemaField> {
    BOARD_OPS
        .iter()
        .flat_map(|spec| spec.args.iter())
        .map(|arg| arg.name)
        .chain([ID_ALIAS.0])
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
///
/// ```
/// use quecto::infrastructure::tools::swarm_board_dispatch::{BindingFaults, unreadable_arguments};
/// let faults = unreadable_arguments(Some(vec!["sk-ant-api03-anything".to_owned()]));
/// assert_eq!(faults.record().unreadable_args, ["arguments"]);
/// assert!(BindingFaults::NONE.is_empty());
/// ```
///
/// Its fields are private, so none is filled from outside,
///
/// ```compile_fail
/// use quecto::infrastructure::tools::swarm_board_dispatch::BindingFaults;
/// let faults = BindingFaults { missing: Vec::new(), ..BindingFaults::NONE };
/// ```
///
/// and it has no deserializer.
///
/// ```compile_fail
/// use quecto::infrastructure::tools::swarm_board_dispatch::BindingFaults;
/// let faults: BindingFaults = serde_json::from_str("{}").unwrap();
/// ```
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

/// The schema type mismatches of arguments that bind (#2346 final review):
/// read before binding takes them, since the board may still refuse a
/// value as `invalid`, and only for a member-facing op ([`op_spec`]); any
/// other method (the harness's own polling) reads nothing.
pub(super) fn mistyped(method: Method, args: &Value) -> BindingFaults {
    match op_spec(method.name()) {
        Some(_) => faults(method, args),
        None => BindingFaults::NONE,
    }
}

fn unexpected(faults: &mut BindingFaults, known: Option<SchemaField>) {
    faults.unexpected += 1;
    faults.unexpected_known.extend(known);
}

/// The faults of a call that gave `field` twice, under two names with two
/// values (#2478: `task_id` and `task_item_id`), as Python's binding
/// refuses an argument given twice: one argument more than the op takes,
/// `field`.
pub fn given_twice(field: SchemaField) -> BindingFaults {
    BindingFaults {
        unexpected: 1,
        unexpected_known: vec![field],
        ..BindingFaults::NONE
    }
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

/// The swarm's tasks are task items (#2478). Each op whose name says task
/// has its task-item name too ([`OP_ALIASES`]), and `task_id` is also
/// taken as `task_item_id` ([`ID_ALIAS`]). The tool reads a task-item name
/// as the old one before anything else (binding, the running gate, the
/// board, refusals, `swarm_op` records), so the board, its events and the
/// golden fixtures keep the old names; each call's records say which names
/// it used ([`name_form`]).
pub mod task_item_names {
    use serde_json::{Map, Value};

    use super::{BindingFaults, given_twice, schema_field};
    use crate::domain::swarm::NameForm;
    use crate::infrastructure::tools::swarm_board_ops::OpSpec;

    /// `(task-item name, old name)` for each op whose name says task.
    pub const OP_ALIASES: &[(&str, &str)] = &[
        ("task_item", "task"),
        ("task_items", "tasks"),
        ("task_item_create", "task_create"),
        ("verify_task_item", "verify_task"),
        ("revalidate_task_item", "revalidate_task"),
    ];

    /// `task_id`'s task-item name, `(task-item name, old name)`: one argument
    /// under two names, on every op that takes it.
    pub const ID_ALIAS: (&str, &str) = ("task_item_id", "task_id");

    /// The op `name` names: the old name a task-item name stands for, or
    /// `name` itself.
    pub fn canonical_op(name: &str) -> &str {
        match OP_ALIASES.iter().find(|(alias, _)| *alias == name) {
            Some((_, old)) => old,
            None => name,
        }
    }

    /// Whether `spec` takes `task_id`.
    pub fn takes_task_id(spec: &OpSpec) -> bool {
        spec.args.iter().any(|arg| arg.name == ID_ALIAS.1)
    }

    /// The argument `key`, given to `spec`, binds as: `task_id` for
    /// `task_item_id` when `spec` takes `task_id`; any other key (and
    /// `task_item_id` sent to an op that takes no task id, which the binding
    /// then refuses under the name it was sent as) as itself.
    pub fn canonical_key<'key>(spec: &OpSpec, key: &'key str) -> &'key str {
        match key == ID_ALIAS.0 && takes_task_id(spec) {
            true => ID_ALIAS.1,
            false => key,
        }
    }

    /// A call that gave `task_id` and `task_item_id` different values: one
    /// argument given twice, which Python's binding refuses as a call error.
    pub struct GivenTwice {
        op: &'static str,
    }

    impl GivenTwice {
        /// The refusal's text, shaped as the binding's own (`<op>: ...`).
        pub fn text(&self) -> String {
            let (alias, old) = ID_ALIAS;
            format!(
                "{}: {old} and {alias} name the same argument but hold different values; send only one",
                self.op
            )
        }

        /// What the refusal found wrong: `task_id` given once more than the op
        /// takes it.
        pub fn faults(&self) -> BindingFaults {
            let field = schema_field(ID_ALIAS.1).expect("task_id is a schema field");
            given_twice(field)
        }
    }

    /// `fields`, given to `spec`, with `task_item_id` read as `task_id` in its
    /// place (the binding names the first unexpected field in the member's
    /// order), when `spec` takes `task_id`; as given otherwise.
    ///
    /// # Errors
    /// Both names given different values (`1` and `1.0` differ, as `null` and
    /// `1` do): one argument cannot hold two.
    pub fn canonical_fields(
        spec: &'static OpSpec,
        fields: Map<String, Value>,
    ) -> Result<Map<String, Value>, GivenTwice> {
        let (alias, old) = ID_ALIAS;
        match (takes_task_id(spec), fields.get(alias), fields.get(old)) {
            (false, _, _) | (true, None, _) => Ok(fields),
            (true, Some(given), Some(same)) if given == same => {
                let mut fields = fields;
                fields.shift_remove(alias);
                Ok(fields)
            }
            (true, Some(_), Some(_)) => Err(GivenTwice { op: spec.name }),
            (true, Some(_), None) => Ok(fields
                .into_iter()
                .map(|(key, value)| (canonical_key(spec, &key).to_owned(), value))
                .collect()),
        }
    }

    /// Which names a call used, where any applies (#2478): `asked`, the op
    /// name it gave for `spec`, when `spec` was renamed, and `task_id` or
    /// `task_item_id` among `fields` (its arguments, when they were read as an
    /// object). [`NameForm::Legacy`] when it used any old name, so a call that
    /// would fail once the old names are retired counts as one;
    /// [`NameForm::Current`] when it used task-item names only; `None` when it
    /// used neither.
    pub fn name_form(
        asked: &str,
        spec: &OpSpec,
        fields: Option<&Map<String, Value>>,
    ) -> Option<NameForm> {
        let renamed = OP_ALIASES.iter().any(|(_, old)| *old == spec.name);
        let op = renamed.then_some(match asked == spec.name {
            true => NameForm::Legacy,
            false => NameForm::Current,
        });
        let given = |name: &str| fields.is_some_and(|fields| fields.contains_key(name));
        let old_id = given(ID_ALIAS.1).then_some(NameForm::Legacy);
        let new_id = given(ID_ALIAS.0).then_some(NameForm::Current);
        [op, old_id, new_id]
            .into_iter()
            .flatten()
            .reduce(|first, second| match (first, second) {
                (NameForm::Current, NameForm::Current) => NameForm::Current,
                (NameForm::Legacy, _) | (_, NameForm::Legacy) => NameForm::Legacy,
            })
    }
}

#[cfg(test)]
#[path = "swarm_board_dispatch_binding_tests.rs"]
mod tests;
