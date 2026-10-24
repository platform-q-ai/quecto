//! Who is asking over TCP (epic #2705 B5): each source's unauthenticated
//! connections, and [`Answers`], the blocking answers TCP callers hold.
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

/// The key a TCP peer is counted under: all of loopback (127.0.0.0/8 and
/// ::1, which any local user can bind) is one source.
pub(super) fn source_of(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) if v4.is_loopback() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_loopback() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V4(_) | IpAddr::V6(_) => ip,
    }
}

/// A connection's signals: `evicted` is told when newer ones push it out;
/// `heard` is set once it sends a byte.
#[derive(Debug, Default)]
pub(super) struct Signals {
    pub(super) evicted: Notify,
    pub(super) heard: Arc<AtomicBool>,
}

type Queues = HashMap<IpAddr, VecDeque<(u64, Arc<Signals>)>>;

/// One listener's sources.
#[derive(Debug, Clone, Default)]
pub(super) struct Sources(Arc<Mutex<(u64, Queues)>>);

/// A connection's place in its source's queue, given up when dropped.
#[derive(Debug)]
pub(super) struct Unproven {
    sources: Sources,
    source: IpAddr,
    id: u64,
    pub(super) signals: Arc<Signals>,
}

impl Unproven {
    /// Whether it came over loopback.
    pub(super) fn loopback(&self) -> bool {
        self.source.is_loopback()
    }
}

impl Sources {
    /// A place for a connection from `source`; past `most`, the oldest
    /// silent connection (else the oldest) is evicted.
    pub(super) fn enter(&self, source: IpAddr, most: usize) -> Unproven {
        // Each change leaves the map whole, so a poisoned lock is as good.
        let mut open = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = open.0;
        open.0 = id.wrapping_add(1);
        let queue = open.1.entry(source).or_default();
        while queue.len() >= most.max(1) {
            let heard = |(_, signals): &(u64, Arc<Signals>)| signals.heard.load(Ordering::Relaxed);
            let silent = queue.iter().position(|entry| !heard(entry)).unwrap_or(0);
            match queue.remove(silent) {
                Some((_, evicted)) => evicted.evicted.notify_one(),
                None => break,
            }
        }
        let signals = Arc::new(Signals::default());
        queue.push_back((id, Arc::clone(&signals)));
        let sources = self.clone();
        Unproven {
            sources,
            source,
            id,
            signals,
        }
    }
}

impl Drop for Unproven {
    fn drop(&mut self) {
        let mut open = self
            .sources
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(queue) = open.1.get_mut(&self.source) {
            queue.retain(|(id, _)| *id != self.id);
            if queue.is_empty() {
                open.1.remove(&self.source);
            }
        }
    }
}

/// The blocking answers TCP requests may hold at once: in all, and each
/// principal.
#[derive(Debug)]
pub(super) struct Answers {
    all: Arc<Semaphore>,
    each: Mutex<HashMap<String, Arc<Semaphore>>>,
    per: usize,
}

impl Answers {
    pub(super) fn new(all: usize, per: usize) -> Self {
        let all = Arc::new(Semaphore::new(all));
        Self {
            all,
            each: Mutex::default(),
            per,
        }
    }

    /// Room for one more answer for `principal`, held while the permits
    /// live; `None` when it, or every TCP caller together, is at its most.
    pub(super) fn admit(&self, principal: &str) -> Option<[OwnedSemaphorePermit; 2]> {
        let mut each = self.each.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = || Arc::new(Semaphore::new(self.per));
        let mine = Arc::clone(each.entry(principal.to_string()).or_insert_with(fresh));
        drop(each);
        let mine = mine.try_acquire_owned().ok()?;
        Some([mine, Arc::clone(&self.all).try_acquire_owned().ok()?])
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
