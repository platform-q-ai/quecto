use super::*;
use quecto::domain::session::{SpillEntry, SpillIndex, SpillIndexList};
use quecto::infrastructure::tools::recall::RecallTool;

#[derive(Debug, Default)]
pub struct BddMemorySpillStore {
    entries_by_session: Mutex<HashMap<String, Vec<SpillEntry>>>,
}

impl BddMemorySpillStore {
    fn add_entry(
        &self,
        session_key: &str,
        id: String,
        tool: String,
        input_preview: String,
        content: String,
    ) {
        self.entries_by_session
            .lock()
            .unwrap()
            .entry(session_key.to_string())
            .or_default()
            .push(SpillEntry {
                id,
                tool,
                input_preview,
                tokens: 123,
                content,
            });
    }
}

impl ContextSpillStore for BddMemorySpillStore {
    fn append(
        &self,
        session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries_by_session
            .lock()
            .unwrap()
            .entry(session_key.to_string())
            .or_default()
            .push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let result = self
            .entries_by_session
            .lock()
            .unwrap()
            .get(session_key)
            .and_then(|entries| entries.iter().find(|entry| entry.id == id).cloned());
        Box::pin(async move { Ok(result) })
    }

    fn list_entries(&self, session_key: &str) -> SpillIndexList<'_> {
        let entries = self
            .entries_by_session
            .lock()
            .unwrap()
            .get(session_key)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|entry| SpillIndex {
                id: entry.id,
                tool: entry.tool,
                input_preview: entry.input_preview,
                tokens: entry.tokens,
            })
            .collect::<Vec<_>>();
        Box::pin(async move { Ok(Arc::new(entries)) })
    }

    fn clear(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries_by_session.lock().unwrap().remove(session_key);
        Box::pin(async { Ok(()) })
    }
}

fn recall_store(world: &QuectoWorld) -> Arc<BddMemorySpillStore> {
    world
        .recall_spill_store
        .as_ref()
        .expect("recall spill store not set")
        .clone()
}

fn execute_recall(world: &mut QuectoWorld, id: &str) {
    let tool = world.recall_tool.as_ref().expect("recall tool not set");
    let args = serde_json::json!({ "id": id }).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(&args))
        .expect("recall tool execution should not raise a domain error");
    world.recall_result = Some(result);
}

#[given(expr = "a recall tool for session {string} with no spilled outputs")]
fn given_recall_tool_with_no_spills(world: &mut QuectoWorld, session_key: String) {
    let store = Arc::new(BddMemorySpillStore::default());
    world.recall_spill_store = Some(store.clone());
    world.recall_tool = Some(RecallTool::new(store, session_key));
    world.recall_result = None;
}

#[given(
    expr = "a recall tool for session {string} with spilled output {string} from tool {string} preview {string} containing {string}"
)]
fn given_recall_tool_with_spilled_output(
    world: &mut QuectoWorld,
    session_key: String,
    id: String,
    tool: String,
    input_preview: String,
    content: String,
) {
    given_recall_tool_with_no_spills(world, session_key.clone());
    recall_store(world).add_entry(&session_key, id, tool, input_preview, content);
}

#[given(
    expr = "session {string} has spilled output {string} from tool {string} preview {string} containing {string}"
)]
fn given_session_has_spilled_output(
    world: &mut QuectoWorld,
    session_key: String,
    id: String,
    tool: String,
    input_preview: String,
    content: String,
) {
    recall_store(world).add_entry(&session_key, id, tool, input_preview, content);
}

#[when(expr = "I switch the recall tool to session {string}")]
fn when_switch_recall_tool_session(world: &mut QuectoWorld, session_key: String) {
    let tool = world.recall_tool.as_ref().expect("recall tool not set");
    tool.set_session_key(session_key);
}

#[when(expr = "I run recall with id {string}")]
fn when_i_run_recall_with_id(world: &mut QuectoWorld, id: String) {
    execute_recall(world, &id);
}

#[then("the recall result should not be an error")]
fn then_recall_result_should_not_be_error(world: &mut QuectoWorld) {
    let result = world.recall_result.as_ref().expect("recall result not set");
    assert!(
        !result.is_error,
        "expected recall success, got error: {}",
        result.content
    );
}

#[then("the recall result should be an error")]
fn then_recall_result_should_be_error(world: &mut QuectoWorld) {
    let result = world.recall_result.as_ref().expect("recall result not set");
    assert!(
        result.is_error,
        "expected recall error, got success: {}",
        result.content
    );
}

#[then(expr = "the recall result should contain {string}")]
fn then_recall_result_should_contain(world: &mut QuectoWorld, expected: String) {
    let result = world.recall_result.as_ref().expect("recall result not set");
    assert!(
        result.content.contains(&expected),
        "expected recall result to contain '{}', got: {}",
        expected,
        result.content
    );
}

#[then(expr = "the recall result should not contain {string}")]
fn then_recall_result_should_not_contain(world: &mut QuectoWorld, unexpected: String) {
    let result = world.recall_result.as_ref().expect("recall result not set");
    assert!(
        !result.content.contains(&unexpected),
        "expected recall result not to contain '{}', got: {}",
        unexpected,
        result.content
    );
}
