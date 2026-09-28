// The `recall` tool's results: the index, and the refusal of an id no
// entry carries, which gives a way forward (#2215).

use crate::domain::session::SpillEntries;
use crate::domain::tool::ToolResult;
use crate::infrastructure::tools::filesystem::{
    MAX_READ_BYTES, bash_paging_example, read_cap_text,
};

/// Where valid ids are listed, and what they look like.
const LIST_HINT: &str =
    "Use recall(\"list\") to see the ids this session has (they look like turn12:bash:0).";

/// The result for an id no retained entry carries (or a malformed id): a
/// way forward (#2215). An id that is a file path, such as the file bash
/// saved a long output to, is sent to `read` when it is within read's cap,
/// else to bounded paging with bash; with no file there to size, both
/// (#2254 review).
pub(super) async fn not_found(id: &str) -> ToolResult {
    let size = match looks_like_file_path(id) {
        true => tokio::fs::metadata(id)
            .await
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map(|meta| meta.len()),
        false => None,
    };
    ToolResult {
        content: not_found_text(id, size),
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

fn not_found_text(id: &str, size: Option<u64>) -> String {
    if !looks_like_file_path(id) {
        return format!("No spilled output found for id: {id}. {LIST_HINT}");
    }
    // The id quoted as a JSON string, so the example is valid JSON.
    let path = serde_json::Value::from(id);
    let read = format!(r#"{{"path":{path},"offset":1,"limit":200}}"#);
    let cap = read_cap_text();
    let open = match size {
        Some(len) if len <= MAX_READ_BYTES => format!("so open it with read, e.g. {read}"),
        Some(_) => format!(
            "and it is over read's {cap} limit, so page through it with bash, e.g. {}",
            bash_paging_example(id, true)
        ),
        None => format!(
            "so open it with read, e.g. {read}, or, if it is over read's {cap} limit, page \
             through it with bash, e.g. {}",
            bash_paging_example(id, false)
        ),
    };
    format!(
        "No spilled output found for id: {id}. {id} looks like a file path: saved bash output \
         is a file, {open}. {LIST_HINT}"
    )
}

/// A path the model may have taken for an id: absolute, or naming bash's
/// saved-output directory. Spill ids (`turn12:bash:0`) are neither.
pub(super) fn looks_like_file_path(id: &str) -> bool {
    id.starts_with('/') || id.contains("/quecto-bash-output")
}

/// The result for `recall("list")`: the index, one line per entry in the
/// use case's order.
pub(super) fn index_result(entries: &SpillEntries) -> ToolResult {
    if entries.is_empty() {
        return ToolResult {
            content: "No spilled outputs in this session.".to_string(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        };
    }
    let mut output = format!("Spilled outputs ({} entries):\n", entries.len());
    for entry in entries.iter() {
        output.push_str(&format!(
            "  {} — {} ({} tokens)\n",
            entry.id, entry.input_preview, entry.tokens
        ));
    }
    ToolResult {
        content: output,
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}
