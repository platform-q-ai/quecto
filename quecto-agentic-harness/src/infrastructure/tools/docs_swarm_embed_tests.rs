//! #2280 (E1-S15): the swarm embed (`docs {"name":"swarm"}`) is the members'
//! API reference, so it teaches only the structured board ops, gives every op
//! of `BOARD_OPS` a runnable example whose fields are exactly the op's
//! arguments with values of the schema's types, states the member-input rule,
//! pins the owner-idle threshold to its constant, and stays self-contained.
use serde_json::{Map, Value};

use super::lookup_doc;
use crate::domain::swarm::owner::OWNER_IDLE_AFTER;
use crate::infrastructure::tools::swarm_board_ops::{ArgSpec, BOARD_OPS, OpSpec};

fn embed() -> &'static str {
    lookup_doc("swarm").expect("the swarm embed")
}

/// Replaces the Python `test_the_idle_threshold_prose_matches_the_constant`,
/// so the pin survives the Python board's deletion (S18).
#[test]
fn the_swarm_embed_states_the_idle_threshold_constant() {
    assert!(OWNER_IDLE_AFTER.fract() == 0.0 && OWNER_IDLE_AFTER > 0.0);
    let prose = format!("{} s", OWNER_IDLE_AFTER as u64);
    assert!(embed().contains(&prose), "the swarm embed misses `{prose}`");
}

#[test]
fn the_swarm_embed_teaches_only_structured_ops() {
    let embed = embed();
    for retired in [
        "from swarm import board",
        "\"op\":\"run\"",
        "op=run",
        "board.",
    ] {
        assert!(
            !embed.contains(retired),
            "the swarm embed still teaches `{retired}`"
        );
    }
    for op in ["claim", "submit", "send", "verify_task"] {
        let example = format!("\"op\":\"{op}\"");
        assert!(embed.contains(&example), "the swarm embed misses {example}");
    }
}

/// The example of `op` in the embed: the backticked JSON object that starts
/// `{"op":"<op>"` followed by a field or the object's end.
fn example_of(embed: &str, op: &str) -> Option<Value> {
    let start = format!("`{{\"op\":\"{op}\"");
    embed.match_indices(&start).find_map(|(index, _)| {
        let rest = &embed[index + 1..];
        let end = rest.find('`')?;
        let text = &rest[..end];
        match text.as_bytes().get(start.len() - 1) {
            Some(b',') | Some(b'}') => serde_json::from_str(text).ok(),
            _ => None,
        }
    })
}

/// Whether `value` is of the JSON schema `schema` (the subset the board
/// ops' schemas use: type or types, enum, items, properties, required).
fn conforms(value: &Value, schema: &Value) -> bool {
    let typed = match &schema["type"] {
        Value::String(name) => of_type(value, name, schema),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| of_type(value, name, schema)),
        _ => false,
    };
    let listed = match &schema["enum"] {
        Value::Array(allowed) => allowed.contains(value),
        _ => true,
    };
    typed && listed
}

fn of_type(value: &Value, name: &str, schema: &Value) -> bool {
    match name {
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "array" => value.as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| schema.get("items").is_none_or(|of| conforms(item, of)))
        }),
        "object" => value.as_object().is_some_and(|fields| {
            let required = schema["required"].as_array().cloned().unwrap_or_default();
            let properties = schema["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            required
                .iter()
                .filter_map(Value::as_str)
                .all(|key| fields.contains_key(key))
                && fields.iter().all(|(key, field)| {
                    properties
                        .get(key)
                        .is_some_and(|property| conforms(field, property))
                })
        }),
        _ => false,
    }
}

fn argument_conforms(arg: &ArgSpec, value: &Value) -> bool {
    let schema: Value = serde_json::from_str(arg.json_type).expect("a schema fragment");
    (value.is_null() && arg.default == Some("null")) || conforms(value, &schema)
}

#[test]
fn the_type_check_tells_the_schema_types_apart() {
    let integers: Value =
        serde_json::from_str(r#"{"type":"array","items":{"type":"integer"}}"#).expect("schema");
    assert!(conforms(&serde_json::json!([1, 2]), &integers));
    assert!(!conforms(&serde_json::json!([1, "2"]), &integers));
    assert!(!conforms(
        &serde_json::json!(true),
        &serde_json::json!({"type":"integer"})
    ));
    let kind = serde_json::json!({"type":"string","enum":["command","review"]});
    assert!(conforms(&serde_json::json!("review"), &kind));
    assert!(!conforms(&serde_json::json!("other"), &kind));
    let nullable = serde_json::json!({"type":["integer","null"]});
    assert!(conforms(&Value::Null, &nullable));
    let evidence = serde_json::json!({"type":"object","properties":{"artifact":{"type":"string"}},
        "required":["artifact"]});
    assert!(conforms(&serde_json::json!({"artifact":"a"}), &evidence));
    assert!(!conforms(&serde_json::json!({}), &evidence));
    assert!(!conforms(
        &serde_json::json!({"artifact":"a","x":1}),
        &evidence
    ));
}

/// Every member op has one runnable example: exactly its arguments (the
/// defaulted ones shown at a value of their type), each of the schema's
/// type, so a member copying it sends no wrong field (the issue's risk).
#[test]
fn the_swarm_embed_lists_every_member_op() {
    let embed = embed();
    let mut problems = Vec::new();
    for spec in BOARD_OPS {
        let Some(example) = example_of(embed, spec.name) else {
            problems.push(format!(
                "{}: no `{{\"op\":\"{}\",...}}` example",
                spec.name, spec.name
            ));
            continue;
        };
        let fields = example.as_object().expect("an example is an object");
        let mut named: Vec<&str> = fields.keys().map(String::as_str).collect();
        named.sort_unstable();
        let mut expected: Vec<&str> = spec.args.iter().map(|arg| arg.name).collect();
        expected.push("op");
        expected.sort_unstable();
        if named != expected {
            problems.push(format!(
                "{}: fields {named:?}, arguments {expected:?}",
                spec.name
            ));
        }
        problems.extend(type_problems(spec, fields));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The problems of each field of `fields` that `spec` takes: a value not of
/// the argument's schema type.
fn type_problems(spec: &OpSpec, fields: &Map<String, Value>) -> Vec<String> {
    spec.args
        .iter()
        .filter_map(|arg| match fields.get(arg.name) {
            Some(value) if argument_conforms(arg, value) => None,
            Some(value) => Some(format!(
                "{}.{}: {value} is not {}",
                spec.name, arg.name, arg.json_type
            )),
            None => None,
        })
        .collect()
}

/// Every line of every fenced ```json block in `embed` that is a board op:
/// its op's spec and its fields. Lines naming a harness op are left out.
fn fenced_board_ops(embed: &str) -> Vec<(&'static OpSpec, Map<String, Value>)> {
    let mut found = Vec::new();
    for block in embed.split("```json\n").skip(1) {
        let body = block.split("```").next().unwrap_or_default();
        for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("fenced line `{line}` is no JSON: {error}"));
            let Value::Object(fields) = value else {
                panic!("fenced line `{line}` is no JSON object");
            };
            let op = fields["op"].as_str().expect("a fenced op names its op");
            if let Some(spec) = BOARD_OPS.iter().find(|spec| spec.name == op) {
                found.push((spec, fields));
            }
        }
    }
    found
}

/// The problems of one fenced op: a field the op does not take, a required
/// argument left out, or a value not of the schema's type. Defaulted
/// arguments may be left out, as a member may leave them out.
fn fenced_problems(spec: &OpSpec, fields: &Map<String, Value>) -> Vec<String> {
    let mut problems: Vec<String> = fields
        .keys()
        .filter(|key| key.as_str() != "op")
        .filter(|key| !spec.args.iter().any(|arg| arg.name == key.as_str()))
        .map(|key| format!("{}: takes no field {key}", spec.name))
        .collect();
    problems.extend(
        spec.args
            .iter()
            .filter(|arg| arg.required && !fields.contains_key(arg.name))
            .map(|arg| format!("{}: misses required {}", spec.name, arg.name)),
    );
    problems.extend(type_problems(spec, fields));
    problems
}

/// The fenced working sequences are copied as whole calls, so each of their
/// board ops is held to the schema too, not only the first example per op.
#[test]
fn the_swarm_embed_fenced_sequences_match_the_schema() {
    let ops = fenced_board_ops(embed());
    assert!(!ops.is_empty(), "the swarm embed has no fenced board op");
    let problems: Vec<String> = ops
        .iter()
        .flat_map(|(spec, fields)| fenced_problems(spec, fields))
        .collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_fenced_check_tells_a_wrong_call_apart() {
    let sample = "```json\n{\"op\":\"claim\",\"task_id\":true}\n\
                  {\"op\":\"claim\",\"id\":1}\n{\"op\":\"summary\"}\n```\n";
    let ops = fenced_board_ops(sample);
    assert_eq!(ops.len(), 2, "the harness op `summary` is left out");
    let problems: Vec<String> = ops
        .iter()
        .flat_map(|(spec, fields)| fenced_problems(spec, fields))
        .collect();
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("task_id: true")));
    assert!(problems.iter().any(|p| p.contains("takes no field id")));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("misses required task_id"))
    );
}

/// S14's member-input rule: text the board's value type cannot hold exactly
/// is refused before the board, and the embed says which values those are.
#[test]
fn the_swarm_embed_states_the_member_input_rule() {
    let embed = embed();
    for needle in [
        "i64",
        "u64",
        "NaN",
        "Infinity",
        "lone surrogate",
        "arguments: ",
    ] {
        assert!(embed.contains(needle), "the swarm embed misses {needle}");
    }
}

/// The embed is served where no documentation files exist: it names no
/// other file to read.
#[test]
fn the_swarm_embed_is_self_contained() {
    let embed = embed();
    for reference in ["](", ".md", "docs/"] {
        assert!(
            !embed.contains(reference),
            "the swarm embed refers to another file: `{reference}`"
        );
    }
}

/// The words the swarm docs write as an `agent_cmd` command: each
/// `agent_cmd <word>` or `` agent_cmd` `<word> `` names its next word, a
/// run of lowercase letters and underscores.
fn named_agent_cmd_commands(doc: &str) -> Vec<String> {
    ["agent_cmd ", "agent_cmd` `"]
        .iter()
        .flat_map(|lead| doc.match_indices(lead).map(move |(at, _)| at + lead.len()))
        .map(|start| {
            doc[start..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                .collect::<String>()
        })
        // A JSON call (`agent_cmd {"command":…}`) names no word here.
        .filter(|word| !word.is_empty())
        .collect()
}

/// A swarm run's report (2026-10-01) caught the guide naming
/// `agent_cmd status`, which is not a command: every command the swarm docs
/// name must be one `agent_cmd` accepts.
#[test]
fn the_swarm_docs_name_only_real_agent_cmd_commands() {
    let pages = [
        ("the swarm embed", embed()),
        ("docs/swarm.md", include_str!("../../../docs/swarm.md")),
    ];
    for (page, doc) in pages {
        let named = named_agent_cmd_commands(doc);
        assert!(!named.is_empty(), "{page} names no agent_cmd command");
        for command in named {
            assert!(
                crate::infrastructure::tools::agent_cmd_parse::SUPPORTED_COMMANDS
                    .contains(&command.as_str()),
                "{page} names `agent_cmd {command}`, which is not a command"
            );
        }
    }
}
