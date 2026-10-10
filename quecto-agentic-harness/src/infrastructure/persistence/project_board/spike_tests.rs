use std::path::Path;
use std::process::Command;

use super::git_board::GitBoard;
use super::index;
use crate::application::project_board::ports::{ProjectBoardRepository, PublishError};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::transition;
use crate::domain::project_board::value_objects::status::{Identity, TaskKind, TaskStatus};

const ENV: [(&str, &str); 2] = [("GIT_CONFIG_GLOBAL", "/dev/null"), ("GIT_CONFIG_NOSYSTEM", "1")];

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).envs(ENV)
        .env("GIT_AUTHOR_NAME", "u").env("GIT_AUTHOR_EMAIL", "u@x").env("GIT_COMMITTER_NAME", "u").env("GIT_COMMITTER_EMAIL", "u@x")
        .output().unwrap();
    assert!(out.status.success(), "{args:?} {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn board(repo: &Path, who: &str) -> GitBoard {
    GitBoard { repo: repo.into(), remote: "origin".into(), committer: Identity { name: who.into(), email: format!("{who}@x") },
        env: ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
}

fn task(id: &str) -> Task {
    Task { id: id.into(), title: "Do it".into(), kind: TaskKind::Task, description: String::new(), status: TaskStatus::Ready,
        parent: None, children: vec![], depends_on: vec![], claim: None, pr: None, created: 1, updated: 1 }
}

fn claim_with_retry(b: &GitBoard, id: &str, now: u64) -> Result<String, String> {
    for _ in 0..5 {
        let snap = b.snapshot().map_err(|e| format!("{e:?}"))?;
        let t = snap.tasks.iter().find(|t| t.id == id).ok_or("no task")?;
        let next = transition::claim(t, &b.committer, now)?;
        match b.publish(&snap.head, &[next], &format!("claim {id}")) {
            Ok(head) => return Ok(head),
            Err(PublishError::Conflict) => continue,
            Err(e) => return Err(format!("{e:?}")),
        }
    }
    Err("retries exhausted".into())
}

#[test]
fn spike_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    sh(tmp.path(), &["init", "-q", "--bare", remote.to_str().unwrap()]);
    let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
    for dir in [&a, &b] {
        sh(tmp.path(), &["clone", "-q", remote.to_str().unwrap(), dir.to_str().unwrap()]);
    }
    std::fs::write(a.join("dirty.txt"), "x").unwrap();
    sh(&a, &["add", "dirty.txt"]);
    let index_before = std::fs::read(a.join(".git/index")).unwrap();

    let alice = board(&a, "alice");
    let bob = board(&b, "bob");
    assert!(alice.bootstrap().unwrap());
    assert!(!bob.bootstrap().unwrap(), "second bootstrap is a no-op");
    let snap = alice.snapshot().unwrap();
    alice.publish(&snap.head, &[task("t1")], "add t1").unwrap();

    // Bob reads the old head and loses the CAS against Alice's claim.
    let bob_stale = bob.snapshot().unwrap();
    claim_with_retry(&alice, "t1", 100).unwrap();
    let mut stolen = bob_stale.tasks[0].clone();
    stolen = transition::claim(&stolen, &bob.committer, 100).unwrap();
    assert_eq!(bob.publish(&bob_stale.head, &[stolen], "steal"), Err(PublishError::Conflict));
    // After refetching, the claim is held: refused until it expires.
    assert!(claim_with_retry(&bob, "t1", 200).is_err());
    claim_with_retry(&bob, "t1", 100 + 2 * 3600).unwrap();

    let snap = bob.snapshot().unwrap();
    assert_eq!(snap.tasks[0].claim.as_ref().unwrap().holder.name, "bob");
    let db = tmp.path().join("idx/board.sqlite");
    assert!(index::refresh(&db, &snap).unwrap());
    assert!(!index::refresh(&db, &snap).unwrap(), "unchanged head skips the rebuild");
    assert_eq!(index::by_status(&db, "claimed").unwrap(), vec!["t1".to_string()]);

    // The user's checkout is untouched: index, HEAD, status, branches.
    assert_eq!(std::fs::read(a.join(".git/index")).unwrap(), index_before);
    assert_eq!(sh(&a, &["status", "--porcelain"]), "A  dirty.txt");
    assert_eq!(sh(&a, &["branch", "--format=%(refname)"]), "");
    assert!(!a.join(".git/FETCH_HEAD").exists());
    let json = sh(&remote, &["show", "refs/heads/quecto/board:tasks/t1.json"]);
    eprintln!("{json}");
    eprintln!("{}", sh(&remote, &["log", "--format=%an %s", "refs/heads/quecto/board"]));
}
