//! Model-facing encoding of `agent_cmd get_containers` (#2220).
//!
//! The listing is bounded: by default only what the caller can act on, at
//! most [`MAX_LISTED_ENVIRONMENTS`] rows in ref-number order, each row
//! compact — no create-time `metadata` blob, no hidden ids, the repository
//! once when every row shares it, every text field redacted and clipped —
//! and the whole within [`MAX_LISTING_BYTES`]. What is left out is
//! counted, with a note naming the way to see it.

use crate::domain::environment_listing::{EnvironmentListing, ListedEnvironment, ListingScope};
use crate::domain::environment_registry::{EnvironmentOrigin, EnvironmentRecord};
use crate::domain::redaction::{redact_secrets, redact_url_userinfo};

/// Rows one `get_containers` result carries at most.
pub(super) const MAX_LISTED_ENVIRONMENTS: usize = 20;

/// Bytes one `get_containers` result carries at most. Past it the rows'
/// metadata goes first, then the rows a cap would drop first.
pub(super) const MAX_LISTING_BYTES: usize = 16_384;

/// The record metadata keys a row carries as redacted text, when set: why
/// a box is kept (`retained`) and what a post-mortem inspect found
/// (`cause`, `inspect_status`). Everything else the create script
/// reported repeats the row's fields.
const LISTED_METADATA_KEYS: [&str; 3] = ["retained", "cause", "inspect_status"];

/// The metadata key naming the runtime's container (`quecto-env-<id>`,
/// what its logs are read by): passed through verbatim — never redacted,
/// which could corrupt it — and only when it is a plain container name.
const CONTAINER_KEY: &str = "container";

/// Characters one text field carries at most; longer text is clipped
/// with `…`.
const MAX_TEXT_CHARS: usize = 400;

/// Characters an identifying field (`name`, `config`, `session`) carries
/// at most.
const MAX_LABEL_CHARS: usize = 200;

/// The metadata key a create script names the members' checkout under.
const CHECKOUT_KEY: &str = "checkout";

/// `all` absent, null or `false` lists what the caller can act on; `true`
/// lists every environment on record.
pub(super) fn decode_scope(args: &serde_json::Value) -> Result<ListingScope, String> {
    match args.get("all") {
        None | Some(serde_json::Value::Null) | Some(serde_json::Value::Bool(false)) => {
            Ok(ListingScope::Actionable)
        }
        Some(serde_json::Value::Bool(true)) => Ok(ListingScope::All),
        Some(_) => Err("get_containers all must be a boolean".to_string()),
    }
}

/// `{"containers":[..],"total":N}`, plus `repository` when every row
/// shares one, `hidden`/`omitted` counts with a `note` when rows or their
/// metadata were left out, and `diagnostics` when the inventory may be
/// incomplete (round 2 F-B, #2033). Never more than
/// [`MAX_LISTING_BYTES`]: past it the rows' free-text metadata goes first,
/// then — row by row, the row a cap would drop first first — the
/// container name, then whole rows in that order; a last row that still
/// does not fit is reduced to its `ref` and `status`.
pub(super) fn encode_listing(listing: &EnvironmentListing) -> serde_json::Value {
    encode_listing_within(listing, MAX_LISTING_BYTES)
}

/// [`encode_listing`] within `budget` bytes.
fn encode_listing_within(listing: &EnvironmentListing, budget: usize) -> serde_json::Value {
    let mut trim = Trim {
        rows: listing.shown.iter().collect(),
        free_text: true,
        containers_cut: std::collections::BTreeSet::new(),
        dropped: 0,
        minimal: false,
    };
    loop {
        let result = encode_rows(listing, &trim, budget);
        if result.to_string().len() <= budget {
            return result;
        }
        if trim.minimal {
            // Only a budget below the real one can get here: a
            // `{ref,status}` row and the counts always fit 16 KiB.
            debug_assert!(budget < MAX_LISTING_BYTES, "a minimal row fits the budget");
            return result;
        }
        trim.step();
    }
}

/// How far a listing has been trimmed to fit [`MAX_LISTING_BYTES`].
struct Trim<'a> {
    rows: Vec<&'a ListedEnvironment>,
    /// Rows still carry their free-text metadata keys.
    free_text: bool,
    /// Keep ranks of the rows whose container name was left out.
    containers_cut: std::collections::BTreeSet<usize>,
    /// Rows left out past the cap.
    dropped: usize,
    /// The last row is reduced to `ref` and `status`.
    minimal: bool,
}

impl Trim<'_> {
    /// The next thing to leave out: free text, then one container name,
    /// then one row, then the last row's detail.
    fn step(&mut self) {
        if self.free_text {
            self.free_text = false;
            return;
        }
        let next_container = self
            .rows
            .iter()
            .filter(|row| listed_container(&row.record).is_some())
            .map(|row| row.keep_rank)
            .filter(|rank| !self.containers_cut.contains(rank))
            .max();
        if let Some(rank) = next_container {
            self.containers_cut.insert(rank);
            return;
        }
        if let [_, _, ..] = self.rows.as_slice() {
            let last = self
                .rows
                .iter()
                .enumerate()
                .max_by_key(|(_, row)| row.keep_rank)
                .map(|(at, _)| at)
                .expect("more than one row is left");
            self.rows.remove(last);
            self.dropped += 1;
            return;
        }
        self.minimal = true;
    }
}

fn encode_rows(listing: &EnvironmentListing, trim: &Trim<'_>, budget: usize) -> serde_json::Value {
    let shared = match trim.minimal {
        true => None,
        false => shared_repository(&trim.rows),
    };
    let encoded: Vec<serde_json::Value> = trim
        .rows
        .iter()
        .map(|row| match trim.minimal {
            true => serde_json::json!({
                "ref": clip(&row.record.environment_ref, MAX_LABEL_CHARS),
                "status": row.record.status_label(),
            }),
            false => encode_row(
                row,
                shared.is_some(),
                trim.free_text,
                !trim.containers_cut.contains(&row.keep_rank),
            ),
        })
        .collect();
    let mut result = serde_json::json!({"containers": encoded, "total": listing.total});
    if let Some(repository) = shared {
        result["repository"] = serde_json::json!(repository);
    }
    let omitted = listing.omitted + trim.dropped;
    let mut notes = Vec::new();
    if listing.hidden > 0 {
        result["hidden"] = serde_json::json!(listing.hidden);
        notes.push(format!(
            "{} not joinable (stopped, or other sessions'); \"all\":true lists them, \
             also at most {MAX_LISTED_ENVIRONMENTS}",
            listing.hidden
        ));
    }
    if omitted > 0 {
        // Omitted rows may be stopped ones: only `--all` is sure to show them.
        result["omitted"] = serde_json::json!(omitted);
        notes.push(format!(
            "… {omitted} more (use `quecto container ls --all`)"
        ));
    }
    match (trim.free_text, trim.containers_cut.len(), trim.minimal) {
        (true, 0, false) => {}
        (_, _, true) => notes.push(format!(
            "rows reduced to ref and status to fit {budget} bytes"
        )),
        (_, 0, false) => notes.push(format!(
            "retained/cause/inspect_status left out to fit {budget} bytes"
        )),
        (_, cut, false) => notes.push(format!(
            "retained/cause/inspect_status, and the container name of {cut} row(s), \
             left out to fit {budget} bytes"
        )),
    }
    if let [_, ..] = notes.as_slice() {
        result["note"] = serde_json::json!(notes.join("; "));
    }
    if let [_, ..] = listing.diagnostics.as_slice() {
        let diagnostics: Vec<String> = listing
            .diagnostics
            .iter()
            .map(|line| safe_text(line))
            .collect();
        result["diagnostics"] = serde_json::json!(diagnostics);
    }
    result
}

fn encode_row(
    row: &ListedEnvironment,
    repository_shared: bool,
    with_free_text: bool,
    with_container: bool,
) -> serde_json::Value {
    let record = &row.record;
    let mut value = serde_json::json!({
        "ref": clip(&record.environment_ref, MAX_LABEL_CHARS),
        "status": record.status_label(),
        "config": clip(&safe_label(&record.script_name), MAX_LABEL_CHARS),
        "members": record.members.len(),
        "own": row.own,
        // Clipped, never redacted: it is the path the members work in and
        // the model hands on verbatim; redaction could corrupt it (like a
        // container name), and a path is no place a secret is reported.
        "checkout": clip(&checkout(record), MAX_TEXT_CHARS),
    });
    if let Some(name) = &record.name {
        value["name"] = serde_json::json!(clip(&safe_label(name), MAX_LABEL_CHARS));
    }
    // Another session's row names its creator; a session-less creator
    // (an empty key) is left unnamed rather than shown as "".
    if let (false, Some(creator)) = (row.own, named_session(&record.created_by)) {
        value["session"] = serde_json::json!(clip(&safe_label(creator), MAX_LABEL_CHARS));
    }
    if record.origin == EnvironmentOrigin::Restored {
        // Provenance (#2024 S4d): reachable for a join or a kill, never
        // torn down by a joiner's exit.
        value["restored"] = serde_json::json!(true);
    }
    // Per row only when not named once at the top and not a sandbox.
    if let (false, Some(repository)) = (repository_shared, row_repository(record)) {
        value["repository"] = serde_json::json!(repository);
    }
    if let Some(created_at) = record.created_at {
        value["created_at"] = serde_json::json!(created_at);
    }
    if let Some(last_error) = &record.last_error {
        value["last_error"] = serde_json::json!(safe_text(last_error));
    }
    let mut metadata = match with_free_text {
        true => free_text_metadata(record),
        false => serde_json::Map::new(),
    };
    if let (true, Some(container)) = (with_container, listed_container(record)) {
        metadata.insert(CONTAINER_KEY.to_string(), serde_json::json!(container));
    }
    if metadata.iter().next().is_some() {
        value["metadata"] = serde_json::Value::Object(metadata);
    }
    value
}

fn named_session(key: &str) -> Option<&str> {
    match key {
        "" => None,
        named => Some(named),
    }
}

/// The listed free-text metadata keys, as redacted, clipped text.
fn free_text_metadata(record: &EnvironmentRecord) -> serde_json::Map<String, serde_json::Value> {
    LISTED_METADATA_KEYS
        .iter()
        .filter_map(|key| match record.metadata.get(*key) {
            Some(serde_json::Value::String(text)) => {
                Some((key.to_string(), serde_json::json!(safe_text(text))))
            }
            _ => None,
        })
        .collect()
}

/// The runtime's container name, verbatim, when it is a plain one.
fn listed_container(record: &EnvironmentRecord) -> Option<&str> {
    match record.metadata.get(CONTAINER_KEY) {
        Some(serde_json::Value::String(name)) if is_container_name(name) => Some(name),
        _ => None,
    }
}

/// `^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$`: a name a runtime accepts for a
/// container, and nothing that could carry anything else.
fn is_container_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let first_ok = bytes.next().is_some_and(|b| b.is_ascii_alphanumeric());
    let rest_ok = bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    first_ok && rest_ok && name.len() <= 128
}

/// Free text for the model: URL userinfo and known secret shapes
/// redacted, then clipped to [`MAX_TEXT_CHARS`].
fn safe_text(text: &str) -> String {
    clip(&safe_label(text), MAX_TEXT_CHARS)
}

/// A label for the model, redacted like free text but not yet clipped.
fn safe_label(text: &str) -> String {
    redact_secrets(&redact_url_userinfo(text))
}

fn clip(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// Where the members work: the create script's absolute
/// `metadata.checkout`, else the environment's workspace.
fn checkout(record: &EnvironmentRecord) -> String {
    match record.metadata.get(CHECKOUT_KEY) {
        Some(serde_json::Value::String(path)) if std::path::Path::new(path).is_absolute() => {
            path.clone()
        }
        _ => record.workspace_path.display().to_string(),
    }
}

/// The record's repository as the model may see it (userinfo and secret
/// shapes redacted, clipped); `None` for a sandbox, which names none.
fn row_repository(record: &EnvironmentRecord) -> Option<String> {
    match record.repository.as_str() {
        "" => None,
        repository => Some(safe_text(repository)),
    }
}

/// The one redacted repository every shown row names, when there are at
/// least two rows and they all name the same one.
fn shared_repository(rows: &[&ListedEnvironment]) -> Option<String> {
    let first = row_repository(&rows.first()?.record)?;
    let shared = rows.len() >= 2
        && rows
            .iter()
            .all(|row| row_repository(&row.record).as_deref() == Some(first.as_str()));
    shared.then_some(first)
}

#[cfg(test)]
#[path = "agent_cmd_container_listing_tests.rs"]
mod tests;
