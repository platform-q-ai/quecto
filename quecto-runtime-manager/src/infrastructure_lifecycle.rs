use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) type BoxFutureResult<T> = Pin<Box<dyn Future<Output = Result<T, ManagerError>> + Send>>;

pub trait RuntimeLifecycle: Send + Sync {
    fn start_runtime(
        &self,
        state: AppState,
        body: EnsureRuntimeRequest,
        runtime_ref: String,
        port: u16,
    ) -> BoxFutureResult<ManagedRuntime>;

    fn sync_credentials(&self, state: AppState, credentials_json: String) -> BoxFutureResult<()>;

    fn delete_runtime_pod(&self, state: AppState, pod_name: String) -> BoxFutureResult<()>;

    fn runtime_pod_status(&self, state: AppState, pod_name: String) -> BoxFutureResult<Value>;
}

#[derive(Default)]
pub struct ProductionRuntimeLifecycle;

pub(super) struct PendingStartGuard {
    pending_starts: Arc<Mutex<HashSet<String>>>,
    runtime_ref: String,
    released: AtomicBool,
}

impl PendingStartGuard {
    pub(super) fn new(pending_starts: Arc<Mutex<HashSet<String>>>, runtime_ref: String) -> Self {
        Self {
            pending_starts,
            runtime_ref,
            released: AtomicBool::new(false),
        }
    }

    pub(super) async fn release(&self) {
        self.released.store(true, Ordering::Relaxed);
        self.pending_starts.lock().await.remove(&self.runtime_ref);
    }
}

impl Drop for PendingStartGuard {
    fn drop(&mut self) {
        if self.released.load(Ordering::Relaxed) {
            return;
        }
        let pending_starts = self.pending_starts.clone();
        let runtime_ref = self.runtime_ref.clone();
        tokio::spawn(async move {
            pending_starts.lock().await.remove(&runtime_ref);
        });
    }
}

impl RuntimeLifecycle for ProductionRuntimeLifecycle {
    fn start_runtime(
        &self,
        state: AppState,
        body: EnsureRuntimeRequest,
        runtime_ref: String,
        port: u16,
    ) -> BoxFutureResult<ManagedRuntime> {
        Box::pin(async move { start_runtime(&state, &body, &runtime_ref, port).await })
    }

    fn sync_credentials(&self, state: AppState, credentials_json: String) -> BoxFutureResult<()> {
        Box::pin(async move { patch_credentials_secret(&state, &credentials_json).await })
    }

    fn delete_runtime_pod(&self, state: AppState, pod_name: String) -> BoxFutureResult<()> {
        Box::pin(async move { delete_runtime_pod(&state, &pod_name).await })
    }

    fn runtime_pod_status(&self, state: AppState, pod_name: String) -> BoxFutureResult<Value> {
        Box::pin(async move { runtime_pod_status(&state, &pod_name).await })
    }
}
