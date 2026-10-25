//! Writer barrier on the real runtime destination-creation dependency. Source
//! admission/read transaction must be pinned before create_partial is called.
use super::fixture::*;
use crate::project_board_history_fixture::writer;
use quecto::application::project_board::ports::backup::*;
use quecto::infrastructure::persistence::sqlite::project_board::{
    SqliteProjectBoardStore, backup::SqliteBoardBackup, backup_files::*,
};
use std::{
    fs::File,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

struct WriterAtPartial {
    trigger: Mutex<Option<mpsc::Sender<()>>>,
    done: Mutex<mpsc::Receiver<()>>,
    triggered: AtomicBool,
    real: SystemBackupFileOps,
}
impl BackupFileOps for WriterAtPartial {
    fn create_partial(&self, directory: &File, name: &str) -> io::Result<File> {
        self.triggered.store(true, Ordering::SeqCst);
        self.trigger
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .map_err(io::Error::other)?;
        self.done
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .map_err(io::Error::other)?;
        self.real.create_partial(directory, name)
    }
    fn sync_file(&self, file: &File) -> io::Result<()> {
        self.real.sync_file(file)
    }
    fn publish_noreplace(
        &self,
        directory: &File,
        partial: &AdmittedBackupFile,
        final_name: &str,
    ) -> io::Result<()> {
        self.real.publish_noreplace(directory, partial, final_name)
    }
    fn sync_directory(&self, directory: &File) -> io::Result<()> {
        self.real.sync_directory(directory)
    }
    fn prune_admitted(&self, directory: &File, candidate: &AdmittedBackupFile) -> io::Result<()> {
        self.real.prune_admitted(directory, candidate)
    }
}

#[test]
fn r2_backup_while_writer_runs_is_consistent() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let (go_tx, go_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let path = f.path.clone();
    let thread = std::thread::spawn(move || {
        let store = SqliteProjectBoardStore::open(&path, writer()).unwrap();
        // Independent preflight proves the writer really commits before the
        // missing operation fails, without claiming a backup already exists.
        store
            .fixture_put_raw("tasks/preflight.md", b"writer preflight committed")
            .unwrap();
        ready_tx.send(logical(&path)).unwrap();
        match go_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(()) => {
                for n in 0..4 {
                    store
                        .fixture_put_raw(
                            "tasks/progress.md",
                            format!("writer committed {n}").as_bytes(),
                        )
                        .unwrap();
                }
                done_tx.send(()).unwrap();
                4
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => 0,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                panic!("runtime barrier neither triggered nor cancelled")
            }
        }
    });
    let pinned_view = ready_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("independent writer committed preflight");
    f.assert_wal();
    let files = Arc::new(WriterAtPartial {
        trigger: Mutex::new(Some(go_tx)),
        done: Mutex::new(done_rx),
        triggered: AtomicBool::new(false),
        real: SystemBackupFileOps,
    });
    let adapter = Reached {
        inner: SqliteBoardBackup::with_files(f.root.clone(), files.clone()),
        calls: AtomicUsize::new(0),
    };
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    // On missing-feature/error paths cancel the waiting independent writer;
    // never let an unavailable shell turn this into a timeout/setup failure.
    files.trigger.lock().unwrap().take();
    let committed_during_pin = thread.join().unwrap();
    println!(
        "writer preflight committed; post-pin commits={committed_during_pin}/4; runtime partial barrier triggered={}",
        files.triggered.load(Ordering::SeqCst)
    );
    successful(result);
    assert!(
        files.triggered.load(Ordering::SeqCst),
        "runtime stage after source snapshot pin must trigger writer"
    );
    assert_eq!(
        committed_during_pin, 4,
        "writer progresses while pinned source read transaction is held and before copy completes"
    );
    assert_eq!(
        logical(&f.final_path()),
        pinned_view,
        "backup preserves the pinned pre-writer multi-table committed view, not torn or newer unpinned data"
    );
    integrity(&f.final_path());
}
