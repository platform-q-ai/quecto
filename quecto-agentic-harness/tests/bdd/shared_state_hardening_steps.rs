use super::*;

use quecto::infrastructure::persistence::session_ownership::{
    SessionOwnershipGuard, ownership_stamp_path,
};
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
    pub own_first_guard: Option<SessionOwnershipGuard>,
    pub own_claim: Option<Result<SessionOwnershipGuard, DomainError>>,
    pub own_claimant_pid: Option<u32>,
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

#[when("the stale socket reaper runs with a zero age threshold")]
fn when_reaper_runs_zero_age(world: &mut QuectoWorld) {
    let dir = runtime_dir(world);
    reap_stale_sockets(&dir, std::time::Duration::ZERO);
}

#[when("the stale socket reaper runs with a one day age threshold")]
fn when_reaper_runs_one_day(world: &mut QuectoWorld) {
    let dir = runtime_dir(world);
    reap_stale_sockets(&dir, std::time::Duration::from_secs(86_400));
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

#[given("a credential store whose credentials lock is held by another locker")]
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

#[when(expr = "a credential write for provider {string} is attempted in the background")]
fn when_credential_write_in_background(world: &mut QuectoWorld, provider: String) {
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

#[then("the credential write has not completed while the lock is held")]
fn then_credential_write_blocked(world: &mut QuectoWorld) {
    std::thread::sleep(std::time::Duration::from_millis(300));
    let done = world.hardening.cred_write_done.as_ref().expect("done flag");
    assert!(
        !done.load(Ordering::SeqCst),
        "a credential write must wait for the cross-process credentials lock"
    );
}

#[when("the credentials lock is released")]
fn when_credentials_lock_released(world: &mut QuectoWorld) {
    let lock_file = world.hardening.cred_lock.take().expect("held lock");
    lock_file.unlock().expect("unlock credentials lock file");
    drop(lock_file);
}

#[then(expr = "the credential write completes and provider {string} is stored")]
fn then_credential_write_completes(world: &mut QuectoWorld, provider: String) {
    let writer = world.hardening.cred_writer.take().expect("writer thread");
    writer.join().expect("join background credential write");
    let done = world.hardening.cred_write_done.as_ref().expect("done flag");
    assert!(done.load(Ordering::SeqCst));
    let dir = world.hardening.cred_dir.as_ref().expect("cred dir");
    let store = CredentialStore::new(dir.path());
    let stored = store.get(&provider).expect("read credentials");
    assert!(
        stored.is_some(),
        "provider {provider} must be stored after the lock is released"
    );
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

#[given(expr = "session key {string} is owned by a live process")]
fn given_session_owned_by_live_process(world: &mut QuectoWorld, key: String) {
    let dir = own_dir(world);
    let guard = SessionOwnershipGuard::acquire_as(&dir, &key, std::process::id())
        .expect("first ownership claim must succeed");
    world.hardening.own_first_guard = Some(guard);
}

#[given(expr = "session key {string} is stamped as owned by a dead process")]
fn given_session_stamped_by_dead_process(world: &mut QuectoWorld, key: String) {
    let dir = own_dir(world);
    let stamp = ownership_stamp_path(&dir, &key);
    std::fs::write(&stamp, dead_pid().to_string()).expect("write stale ownership stamp");
}

#[when(expr = "a second process claims ownership of session key {string}")]
fn when_second_process_claims_key(world: &mut QuectoWorld, key: String) {
    let dir = own_dir(world);
    let claimant_pid = std::process::id();
    world.hardening.own_claimant_pid = Some(claimant_pid);
    world.hardening.own_claim = Some(SessionOwnershipGuard::acquire_as(&dir, &key, claimant_pid));
}

#[then("the ownership claim is refused with an error naming the key and owning process")]
fn then_ownership_claim_refused(world: &mut QuectoWorld) {
    let claim = world.hardening.own_claim.as_ref().expect("claim result");
    let err = match claim {
        Err(e) => e.to_string(),
        Ok(_) => panic!("claiming a session key owned by a live process must be refused"),
    };
    let owner_pid = std::process::id().to_string();
    assert!(
        err.contains("shared-key"),
        "refusal must name the session key, got: {err}"
    );
    assert!(
        err.contains(&owner_pid),
        "refusal must name the owning pid, got: {err}"
    );
}

#[then("the ownership claim succeeds and the stamp records the new owner")]
fn then_ownership_claim_succeeds(world: &mut QuectoWorld) {
    let claim = world.hardening.own_claim.as_ref().expect("claim result");
    let guard = match claim {
        Ok(guard) => guard,
        Err(e) => panic!("a stamp left by a dead process must be reclaimable, got: {e}"),
    };
    let claimant_pid = world.hardening.own_claimant_pid.expect("claimant pid");
    let contents = std::fs::read_to_string(guard.stamp_path()).expect("ownership stamp must exist");
    assert!(
        contents.contains(&claimant_pid.to_string()),
        "reclaiming must rewrite the stamp with the new owner pid, got: {contents:?}"
    );
}
