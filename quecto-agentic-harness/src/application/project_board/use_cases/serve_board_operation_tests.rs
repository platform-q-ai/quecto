use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};

use super::*;
use crate::application::project_board::dto::{
    BoardExecution, ListSummaries, OpNoting, ReadRequest, RunEnd,
};
use crate::application::project_board::ports::{
    OperationsOver, ProjectBoardStore, Transacted, WriteId,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// A store keeping answers by write id, counting its transactions.
#[derive(Default)]
struct Kept {
    store: MemoryBoardStore,
    answers: Mutex<BTreeMap<String, String>>,
    transactions: AtomicUsize,
    prepared: Arc<AtomicUsize>,
}

impl BoardOpStore for Kept {
    fn reads(&self) -> Arc<dyn ProjectBoardStore> {
        Arc::new(self.store.client())
    }

    fn kept(&self, _: &OpWrite) -> Result<Option<Transacted>, BoardStoreError> {
        Ok(None)
    }

    fn transact(
        &self,
        write: &OpWrite,
        op: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError> {
        self.transactions.fetch_add(1, Ordering::SeqCst);
        let kept = self
            .answers
            .lock()
            .unwrap()
            .get(write.write_id.as_str())
            .cloned();
        if let Some(answer) = kept {
            return Ok(Transacted::Replayed(answer));
        }
        let answer = op(Arc::new(self.store.client()))?;
        let mut answers = self.answers.lock().unwrap();
        answers.insert(write.write_id.as_str().into(), answer.clone());
        Ok(Transacted::Answered(answer))
    }
}

/// Operations that answer nothing; or none, when `refused`; counting the
/// requests prepared (each a host read).
struct Over {
    refused: bool,
    prepared: Arc<AtomicUsize>,
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
    fn prepare(
        &self,
        _: Option<&BoardRequest>,
        _: &crate::application::project_board::ports::ServedRun,
        _: &dyn ProjectBoardStore,
    ) -> Result<OperationsOver, crate::application::project_board::ports::ServeError> {
        self.prepared.fetch_add(1, Ordering::SeqCst);
        let refused = self.refused;
        Ok(Box::new(move |_| match refused {
            true => Err(ProjectBoardError::NoLauncher),
            false => Ok(Box::new(Nothing) as Box<dyn BoardOperations>),
        }))
    }
}

fn serving(store: &Arc<Kept>, refused: bool) -> ServeBoardOperation {
    ServeBoardOperation::new(
        Arc::clone(store) as Arc<dyn BoardOpStore>,
        Arc::new(Over {
            refused,
            prepared: Arc::clone(&store.prepared),
        }),
    )
}

fn write(n: u8) -> OpWrite {
    OpWrite {
        write_id: WriteId::parse(&format!("{n:08x}-0000-4000-8000-000000000000")).unwrap(),
        request: "cli\n{}".into(),
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
    let mut present = |_: &dyn BoardOperations| {
        Ok(format!(
            "answer {}",
            presented.fetch_add(1, Ordering::SeqCst)
        ))
    };
    let first = serve.execute(Some(&create()), Some(&write(1)), &mut present);
    assert_eq!(first, Ok(Transacted::Answered("answer 0".into())));
    let again = serve.execute(Some(&create()), Some(&write(1)), &mut present);
    assert_eq!(again, Ok(Transacted::Replayed("answer 0".into())));
    let other = serve.execute(Some(&create()), Some(&write(2)), &mut present);
    assert_eq!(other, Ok(Transacted::Answered("answer 1".into())));
}

/// A read takes no transaction; a request no front end could read is
/// presented as a read; a write without its write is refused unrun.
#[test]
fn a_read_takes_no_transaction_and_a_write_needs_its_write() {
    let store = Arc::new(Kept::default());
    let serve = serving(&store, false);
    let mut present = |_: &dyn BoardOperations| Ok("read".to_string());
    assert!(
        serve
            .execute(Some(&list()), Some(&write(1)), &mut present)
            .is_ok()
    );
    assert!(serve.execute(None, None, &mut present).is_ok());
    assert_eq!(store.transactions.load(Ordering::SeqCst), 0);
    let mut never = |_: &dyn BoardOperations| -> Result<String, String> {
        panic!("a write without its write ran")
    };
    assert_eq!(
        serve.execute(Some(&create()), None, &mut never),
        Err(ServeError::NoWrite)
    );
}

/// A retry of a write whose first run still runs is busy, unprepared;
/// once it lands the retry is its kept answer.
#[test]
fn a_write_still_running_is_busy_to_its_retry() {
    let store = Arc::new(Kept::default());
    let serve = Arc::new(serving(&store, false));
    let (entered, release) = (mpsc::channel(), mpsc::channel::<()>());
    let first = {
        let (serve, entered, release) = (Arc::clone(&serve), entered.0, release.1);
        std::thread::spawn(move || {
            let mut present = |_: &dyn BoardOperations| {
                entered.send(()).unwrap();
                let _ = release.recv();
                Ok("first".to_string())
            };
            serve.execute(Some(&create()), Some(&write(1)), &mut present)
        })
    };
    entered.1.recv().unwrap();
    let mut never = |_: &dyn BoardOperations| -> Result<String, String> { panic!("ran twice") };
    assert_eq!(
        serve.execute(Some(&create()), Some(&write(1)), &mut never),
        Err(ServeError::Busy)
    );
    drop(release.0);
    assert_eq!(
        first.join().unwrap(),
        Ok(Transacted::Answered("first".into()))
    );
    let replayed = serve.execute(Some(&create()), Some(&write(1)), &mut never);
    assert_eq!(replayed, Ok(Transacted::Replayed("first".into())));
}

/// Operations that cannot be built, or a store that failed under the
/// presenter, answer nothing: the write keeps no answer, so its retry
/// runs it.
#[test]
fn a_failed_operation_keeps_no_answer() {
    let store = Arc::new(Kept::default());
    let mut present = |_: &dyn BoardOperations| Ok("made".to_string());
    let refused = serving(&store, true).execute(Some(&create()), Some(&write(1)), &mut present);
    assert!(matches!(refused, Err(ServeError::Board(_))), "{refused:?}");
    let mut failing = |_: &dyn BoardOperations| Err("the store failed".to_string());
    let failed = serving(&store, false).execute(Some(&create()), Some(&write(1)), &mut failing);
    assert!(matches!(failed, Err(ServeError::Board(_))), "{failed:?}");
    assert!(store.answers.lock().unwrap().is_empty());
    let read = serving(&store, true).execute(Some(&list()), None, &mut present);
    assert!(matches!(read, Err(ServeError::Board(_))), "{read:?}");
}

/// Past [`MAX_WRITES`] at once, or while its own id still runs, a write
/// is answered busy before it is prepared (no host read), so writes
/// waiting on the lock never take the threads reads need.
#[test]
fn writes_past_the_slots_are_busy() {
    let store = Arc::new(Kept::default());
    let serve = Arc::new(serving(&store, false));
    let (entered, held) = (mpsc::channel(), mpsc::channel::<()>());
    let (entered_tx, release) = (entered.0, Arc::new(Mutex::new(held.1)));
    let holding: Vec<_> = (0..MAX_WRITES)
        .map(|n| {
            let (serve, entered, release) =
                (Arc::clone(&serve), entered_tx.clone(), Arc::clone(&release));
            std::thread::spawn(move || {
                let mut present = |_: &dyn BoardOperations| {
                    entered.send(()).unwrap();
                    let _ = release.lock().unwrap().recv();
                    Ok("held".to_string())
                };
                serve.execute(Some(&create()), Some(&write(n as u8)), &mut present)
            })
        })
        .collect();
    // Every slot holder is inside its write.
    (0..MAX_WRITES).for_each(|_| entered.1.recv().unwrap());
    let mut never = |_: &dyn BoardOperations| -> Result<String, String> { panic!("ran") };
    let prepared = store.prepared.load(Ordering::SeqCst);
    assert_eq!(
        serve.execute(Some(&create()), Some(&write(9)), &mut never),
        Err(ServeError::Busy)
    );
    assert_eq!(
        store.prepared.load(Ordering::SeqCst),
        prepared,
        "busy reads no host"
    );
    assert!(
        serve
            .execute(Some(&list()), None, &mut |_| Ok("read".into()))
            .is_ok()
    );
    drop(held.0);
    for thread in holding {
        assert!(thread.join().unwrap().is_ok());
    }
}
