use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_REGISTRATION_GENERATION: AtomicU64 = AtomicU64::new(1);

pub(crate) fn current_registration_generation() -> u64 {
    NEXT_REGISTRATION_GENERATION.load(Ordering::SeqCst)
}

pub(crate) fn next_registration_generation() -> u64 {
    NEXT_REGISTRATION_GENERATION.fetch_add(1, Ordering::SeqCst)
}
