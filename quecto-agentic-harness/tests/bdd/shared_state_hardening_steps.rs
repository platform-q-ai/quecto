use super::*;

use quecto::infrastructure::persistence::session_ownership::{
    SessionOwnershipGuard, open_stamp_file, ownership_stamp_path,
};
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::interface::cli::uds::reap_stale_sockets;
use std::sync::atomic::{AtomicBool, Ordering};

// #1460 shared-state hardening steps
// ===========================================================================
// Cross-process invariants for the multi-session TUI (epic #1467): liveness-
// probed socket reaping, credentials.json locking, session-key single-writer
// ownership. These drive the real production functions directly.

/// World state for the shared-state hardening scenarios.
#[derive(Debug, Default)]
pub struct HardeningState {
    /// Runtime dir holding the agent sockets under reap.
    pub runtime_dir: Option<TempDir>,
    /// Keeps the live socket accepting for the scenario's duration.
    pub live_listener: Option<std::os::unix::net::UnixListener>,
    pub live_socket: Option<PathBuf>,
    pub dead_socket: Option<PathBuf>,
    /// Credentials-locking scenario state.
    pub cred_dir: Option<TempDir>,
    pub cred_lock: Option<std::fs::File>,
    pub cred_write_done: Option<Arc<AtomicBool>>,
    pub cred_writer: Option<std::thread::JoinHandle<()>>,
    /// Session-ownership scenario state.
    pub own_dir: Option<TempDir>,
    /// Simulated foreign owner: an independent file description holding the
    /// exclusive lock, exactly as another live process would.
    pub own_foreign_lock: Option<std::fs::File>,
    pub own_claim: Option<Result<SessionOwnershipGuard, DomainError>>,
    pub own_claimant_pid: Option<u32>,
    pub own_owner_pid: Option<u32>,
    pub own_key: Option<String>,
    /// Session-store write-path ownership scenario state.
    pub store_dir: Option<TempDir>,
    pub store_lock: Option<std::fs::File>,
    pub store_save_result: Option<Result<(), DomainError>>,
}

fn runtime_dir(world: &mut QuectoWorld) -> PathBuf {
    if world.hardening.runtime_dir.is_none() {
        world.hardening.runtime_dir = Some(TempDir::new().expect("tempdir"));
    }
    world
        .hardening
        .runtime_dir
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf()
}

/// Spawn-and-reap a child so we hold a pid that is guaranteed dead.
fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("true")
        .spawn()
        .expect("spawn true");
    let pid = child.id();
    child.wait().expect("wait true");
    pid
}

/// A live pid that is NOT this process: the parent (test runner). Using the
/// claimant's own pid as the owner would let an error that merely echoes the
/// claimant pass the "names the owning pid" assertions.
fn other_live_pid() -> u32 {
    std::os::unix::process::parent_id()
}

// ─── Socket reaping ─────────────────────────────────────────────────────────

#[given("a live quecto agent socket in a runtime directory")]
fn given_live_agent_socket(world: &mut QuectoWorld) {
    let dir = runtime_dir(world);
    let path = dir.join("quecto-agent-live.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind live socket");
    world.hardening.live_listener = Some(listener);
    world.hardening.live_socket = Some(path);
}

#[given("a dead quecto agent socket file in the runtime directory")]
fn given_dead_agent_socket(world: &mut QuectoWorld) {
    let dir = runtime_dir(world);
    let path = dir.join("quecto-agent-dead.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind dead socket");
    drop(listener); // file remains, nothing accepts
    world.hardening.dead_socket = Some(path);
}

#[when(
    regex = r"^the stale socket reaper runs (treating every socket as past|with every socket well within) the stale age$"
)]
fn when_reaper_runs(world: &mut QuectoWorld, age_relation: String) {
    let dir = runtime_dir(world);
    let max_age = if age_relation.starts_with("treating") {
        // Zero threshold: an mtime heuristic would reap every socket.
        std::time::Duration::ZERO
    } else {
        // Generous threshold: an mtime heuristic would reap none of them.
        std::time::Duration::from_secs(86_400)
    };
    reap_stale_sockets(&dir, max_age);
}

#[then("the live agent socket file still exists")]
fn then_live_socket_exists(world: &mut QuectoWorld) {
    let path = world.hardening.live_socket.as_ref().expect("live socket");
    assert!(
        path.exists(),
        "a socket that accepts connections must never be reaped: {}",
        path.display()
    );
}

#[then("the dead agent socket file has been removed")]
fn then_dead_socket_removed(world: &mut QuectoWorld) {
    let path = world.hardening.dead_socket.as_ref().expect("dead socket");
    assert!(
        !path.exists(),
        "a socket file that no longer accepts connections must be reaped: {}",
        path.display()
    );
}

// ─── credentials.json locking ───────────────────────────────────────────────

#[given("a credential store whose credentials lock is held by another process")]
fn given_credentials_lock_held(world: &mut QuectoWorld) {
    let dir = TempDir::new().expect("tempdir");
    let store = CredentialStore::new(dir.path());
    // Seed so a real file exists to mutate.
    store
        .store(Credential {
            provider: "seed".into(),
            token: "sk-seed".into(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .expect("seed credential");
    // Simulated other process: an independently opened file description
    // holding the exclusive lock (flock semantics are per open description,
    // exactly as another process would hold it).
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(store.lock_path())
        .expect("open lock file");
    lock_file.lock().expect("lock credentials lock file");
    world.hardening.cred_lock = Some(lock_file);
    world.hardening.cred_dir = Some(dir);
}

fn start_background_credential_write(world: &mut QuectoWorld, provider: String) {
    let dir = world
        .hardening
        .cred_dir
        .as_ref()
        .expect("cred dir")
        .path()
        .to_path_buf();
    let done = Arc::new(AtomicBool::new(false));
    let done_writer = Arc::clone(&done);
    let writer = std::thread::spawn(move || {
        let store = CredentialStore::new(&dir);
        store
            .store(Credential {
                provider: provider.clone(),
                token: format!("sk-{provider}"),
                method: AuthMethod::Token,
                expires_at: None,
                refresh_token: None,
                account_id: None,
            })
            .expect("background credential write");
        done_writer.store(true, Ordering::SeqCst);
    });
    world.hardening.cred_write_done = Some(done);
    world.hardening.cred_writer = Some(writer);
}

#[when(expr = "another process attempts a credential write for provider {string}")]
fn when_other_process_writes_credential(world: &mut QuectoWorld, provider: String) {
    start_background_credential_write(world, provider);
}

#[then("the credential write has not completed")]
fn then_credential_write_blocked(world: &mut QuectoWorld) {
    std::thread::sleep(std::time::Duration::from_millis(300));
    let done = world.hardening.cred_write_done.as_ref().expect("done flag");
    assert!(
        !done.load(Ordering::SeqCst),
        "a credential write must wait for the cross-process credentials lock"
    );
}

#[given(expr = "a credential write for provider {string} is blocked by the credentials lock")]
fn given_credential_write_blocked(world: &mut QuectoWorld, provider: String) {
    given_credentials_lock_held(world);
    start_background_credential_write(world, provider);
    then_credential_write_blocked(world);
}

#[when("the credentials lock is released")]
fn when_credentials_lock_released(world: &mut QuectoWorld) {
    let lock_file = world.hardening.cred_lock.take().expect("held lock");
    lock_file.unlock().expect("unlock credentials lock file");
    drop(lock_file);
}

#[then("the credential write completes")]
fn then_credential_write_completes(world: &mut QuectoWorld) {
    let writer = world.hardening.cred_writer.take().expect("writer thread");
    writer.join().expect("join background credential write");
    let done = world.hardening.cred_write_done.as_ref().expect("done flag");
    assert!(done.load(Ordering::SeqCst));
}

#[then(expr = "provider {string} is stored")]
fn then_provider_is_stored(world: &mut QuectoWorld, provider: String) {
    let dir = world.hardening.cred_dir.as_ref().expect("cred dir");
    let store = CredentialStore::new(dir.path());
    let stored = store.get(&provider).expect("read credentials");
    assert!(
        stored.is_some(),
        "provider {provider} must be stored after the lock is released"
    );
}

#[then("the previously stored credentials are still present")]
fn then_seed_credentials_survive(world: &mut QuectoWorld) {
    let dir = world.hardening.cred_dir.as_ref().expect("cred dir");
    let store = CredentialStore::new(dir.path());
    assert!(
        store.get("seed").expect("read credentials").is_some(),
        "a blocked writer must not clobber existing credentials"
    );
}

// ─── Session-key ownership ──────────────────────────────────────────────────

fn own_dir(world: &mut QuectoWorld) -> PathBuf {
    if world.hardening.own_dir.is_none() {
        world.hardening.own_dir = Some(TempDir::new().expect("tempdir"));
    }
    world
        .hardening
        .own_dir
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf()
}

#[given(expr = "session key {string} is owned by another live process")]
fn given_session_owned_by_live_process(world: &mut QuectoWorld, key: String) {
    let dir = own_dir(world);
    // Simulated other live process: an independently opened file description
    // holding the exclusive lock (flock semantics are per open description,
    // exactly as another process would hold it), stamped with the parent's
    // (test runner's) pid so refusals name a pid that is not the claimant.
    let owner_pid = other_live_pid();
    let file = hold_stamp_as(&dir, &key, owner_pid);
    world.hardening.own_foreign_lock = Some(file);
    world.hardening.own_owner_pid = Some(owner_pid);
    world.hardening.own_key = Some(key);
}

/// Lock the ownership stamp for `key` via an independent file description and
/// stamp it with `owner_pid`, simulating a claim held by another live process.
fn hold_stamp_as(sessions_dir: &std::path::Path, key: &str, owner_pid: u32) -> std::fs::File {
    use std::io::Write;
    let file = open_stamp_file(sessions_dir, key).expect("open ownership stamp");
    file.try_lock().expect("foreign owner lock must succeed");
    file.set_len(0).expect("truncate stamp");
    (&file)
        .write_all(owner_pid.to_string().as_bytes())
        .expect("write foreign owner pid");
    file
}

#[given(expr = "session key {string} is stamped as owned by a dead process")]
fn given_session_stamped_by_dead_process(world: &mut QuectoWorld, key: String) {
    let dir = own_dir(world);
    let stamp = ownership_stamp_path(&dir, &key);
    std::fs::write(&stamp, dead_pid().to_string()).expect("write stale ownership stamp");
    world.hardening.own_key = Some(key);
}

#[when(expr = "a second process claims ownership of session key {string}")]
fn when_second_process_claims_key(world: &mut QuectoWorld, key: String) {
    let dir = own_dir(world);
    world.hardening.own_claimant_pid = Some(std::process::id());
    world.hardening.own_claim = Some(SessionOwnershipGuard::acquire(&dir, &key));
}

#[then("the ownership claim is refused with an error naming the key and owning process")]
fn then_ownership_claim_refused(world: &mut QuectoWorld) {
    let claim = world.hardening.own_claim.as_ref().expect("claim result");
    let err = match claim {
        Err(e) => e.to_string(),
        Ok(_) => panic!("claiming a session key owned by a live process must be refused"),
    };
    let key = world.hardening.own_key.as_ref().expect("claimed key");
    let owner_pid = world.hardening.own_owner_pid.expect("owner pid");
    assert!(
        err.contains(key),
        "refusal must name the session key {key:?}, got: {err}"
    );
    assert!(
        err.contains(&owner_pid.to_string()),
        "refusal must name the owning pid {owner_pid}, got: {err}"
    );
}

#[then("the ownership claim succeeds")]
fn then_ownership_claim_succeeds(world: &mut QuectoWorld) {
    let claim = world.hardening.own_claim.as_ref().expect("claim result");
    if let Err(e) = claim {
        panic!("a stamp left by a dead process must be reclaimable, got: {e}");
    }
}

#[then("the ownership stamp records the new owner")]
fn then_stamp_records_new_owner(world: &mut QuectoWorld) {
    let claim = world.hardening.own_claim.as_ref().expect("claim result");
    let guard = claim.as_ref().expect("successful claim");
    let claimant_pid = world.hardening.own_claimant_pid.expect("claimant pid");
    let contents = std::fs::read_to_string(guard.stamp_path()).expect("ownership stamp must exist");
    assert!(
        contents.contains(&claimant_pid.to_string()),
        "reclaiming must rewrite the stamp with the new owner pid, got: {contents:?}"
    );
}

// ─── Session-store write path honors ownership ──────────────────────────────

#[given(expr = "a session store whose key {string} is stamped by another live process")]
fn given_store_key_owned_elsewhere(world: &mut QuectoWorld, key: String) {
    let dir = TempDir::new().expect("tempdir");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let owner_pid = other_live_pid();
    world.hardening.store_lock = Some(hold_stamp_as(&sessions_dir, &key, owner_pid));
    world.hardening.own_owner_pid = Some(owner_pid);
    world.hardening.own_key = Some(key);
    world.hardening.store_dir = Some(dir);
}

#[when(expr = "this process saves a turn for session key {string}")]
async fn when_store_saves_turn(world: &mut QuectoWorld, key: String) {
    let dir = world.hardening.store_dir.as_ref().expect("store dir");
    let store = FileSessionStore::new(dir.path());
    let messages = vec![quecto::domain::message::Message::user("a turn".to_string())];
    world.hardening.store_save_result =
        Some(store.save_clean_delta(&key, &messages, 0, None).await);
}

#[then("the session save is refused with an error naming the key and owning process")]
fn then_session_save_refused(world: &mut QuectoWorld) {
    let result = world
        .hardening
        .store_save_result
        .as_ref()
        .expect("save result");
    let err = match result {
        Err(e) => e.to_string(),
        Ok(()) => panic!("saving a session key owned by another live process must be refused"),
    };
    let key = world.hardening.own_key.as_ref().expect("session key");
    let owner_pid = world.hardening.own_owner_pid.expect("owner pid");
    assert!(
        err.contains(key),
        "refusal must name the session key {key:?}, got: {err}"
    );
    assert!(
        err.contains(&owner_pid.to_string()),
        "refusal must name the owning pid {owner_pid}, got: {err}"
    );
}
