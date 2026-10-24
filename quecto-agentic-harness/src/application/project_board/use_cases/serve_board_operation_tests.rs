use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::application::project_board::dto::{
    BoardExecution, ListSummaries, OpNoting, ReadRequest, RunEnd,
};
use crate::application::project_board::ports::{ProjectBoardStore, Transacted, WriteId};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// A store keeping answers by write id, counting its transactions; it
/// fails each `op` while `failing`.
#[derive(Default)]
struct Kept {
    store: MemoryBoardStore,
    answers: Mutex<BTreeMap<String, String>>,
    transactions: AtomicUsize,
}

impl BoardOpStore for Kept {
    fn reads(&self) -> Arc<dyn ProjectBoardStore> {
        Arc::new(self.store.client())
    }

    fn transact(
        &self,
        write: &OpWrite,
        op: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError> {
        self.transactions.fetch_add(1, Ordering::SeqCst);
        let mut answers = self.answers.lock().unwrap();
        if let Some(answer) = answers.get(write.write_id.as_str()) {
            return Ok(Transacted::Replayed(answer.clone()));
        }
        let answer = op(Arc::new(self.store.client()))?;
        answers.insert(write.write_id.as_str().into(), answer.clone());
        Ok(Transacted::Answered(answer))
    }
}

/// Operations that answer nothing; or none, when `refused`.
struct Over {
    refused: bool,
}

struct Nothing;

impl BoardOperations for Nothing {
    fn execute(&self, _: ReadRequest<'_>, _: ListSummaries, _: OpNoting) -> BoardExecution {
        BoardExecution {
            result: Err(ProjectBoardError::NoLauncher),
            facts: None,
        }
    }

    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        Ok(vec![])
    }
}

impl BoardOperationsOver for Over {
    fn over(
        &self,
        _: Arc<dyn ProjectBoardStore>,
    ) -> Result<Box<dyn BoardOperations>, ProjectBoardError> {
        match self.refused {
            true => Err(ProjectBoardError::NoLauncher),
            false => Ok(Box::new(Nothing)),
        }
    }
}

fn serving(store: &Arc<Kept>, refused: bool) -> ServeBoardOperation {
    ServeBoardOperation::new(
        Arc::clone(store) as Arc<dyn BoardOpStore>,
        Arc::new(Over { refused }),
    )
}

fn write(id: &str) -> OpWrite {
    OpWrite {
        write_id: WriteId::parse(id).unwrap(),
        committer: Identity {
            name: "Ada".into(),
            email: "ada@example.com".into(),
        },
        principal: "local".into(),
    }
}

fn create() -> BoardRequest {
    BoardRequest::Complete(Slug::parse("a").unwrap())
}

fn list() -> BoardRequest {
    BoardRequest::ListReady
}

/// A write is presented once per write id: the same id again is the
/// first answer, presented never again.
#[test]
fn a_write_runs_once_per_write_id() {
    let store = Arc::new(Kept::default());
    let serve = serving(&store, false);
    let presented = AtomicUsize::new(0);
    let mut present =
        |_: &dyn BoardOperations| format!("answer {}", presented.fetch_add(1, Ordering::SeqCst));
    let first = serve.execute(Some(&create()), Some(&write("aaaaaaaa")), &mut present);
    assert_eq!(first, Ok(Transacted::Answered("answer 0".into())));
    let again = serve.execute(Some(&create()), Some(&write("aaaaaaaa")), &mut present);
    assert_eq!(again, Ok(Transacted::Replayed("answer 0".into())));
    let other = serve.execute(Some(&create()), Some(&write("bbbbbbbb")), &mut present);
    assert_eq!(other, Ok(Transacted::Answered("answer 1".into())));
}

/// A read takes no transaction; a request no front end could read is
/// presented as a read; a write without its write is refused unrun.
#[test]
fn a_read_takes_no_transaction_and_a_write_needs_its_write() {
    let store = Arc::new(Kept::default());
    let serve = serving(&store, false);
    let mut present = |_: &dyn BoardOperations| "read".to_string();
    assert!(
        serve
            .execute(Some(&list()), Some(&write("aaaaaaaa")), &mut present)
            .is_ok()
    );
    assert!(serve.execute(None, None, &mut present).is_ok());
    assert_eq!(store.transactions.load(Ordering::SeqCst), 0);
    let mut never = |_: &dyn BoardOperations| -> String { panic!("a write without its write ran") };
    assert_eq!(
        serve.execute(Some(&create()), None, &mut never),
        Err(ServeError::NoWrite)
    );
}

/// Operations that cannot be built answer nothing: the write keeps no
/// answer, so its retry runs it.
#[test]
fn operations_that_cannot_be_built_keep_no_answer() {
    let store = Arc::new(Kept::default());
    let mut present = |_: &dyn BoardOperations| "made".to_string();
    let refused =
        serving(&store, true).execute(Some(&create()), Some(&write("aaaaaaaa")), &mut present);
    assert!(matches!(refused, Err(ServeError::Board(_))), "{refused:?}");
    assert!(store.answers.lock().unwrap().is_empty());
    let read = serving(&store, true).execute(Some(&list()), None, &mut present);
    assert!(matches!(read, Err(ServeError::Board(_))), "{read:?}");
}
