//! Each TCP source's connections not yet authenticated (epic #2705 B5),
//! oldest first. One past the cap evicts the oldest, so idle sockets never
//! lock out a client that authenticates at once.
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Notify;

/// The key a TCP peer is counted under: all of loopback (127.0.0.0/8 and
/// ::1, which any local user can bind) is one source.
pub(super) fn source_of(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) if v4.is_loopback() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_loopback() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V4(_) | IpAddr::V6(_) => ip,
    }
}

type Queues = HashMap<IpAddr, VecDeque<(u64, Arc<Notify>)>>;

/// One listener's sources.
#[derive(Debug, Clone, Default)]
pub(super) struct Sources(Arc<Mutex<(u64, Queues)>>);

/// A connection's place in its source's queue, given up when dropped;
/// `evicted` is told when newer connections push it out.
#[derive(Debug)]
pub(super) struct Unproven {
    sources: Sources,
    source: IpAddr,
    id: u64,
    pub(super) evicted: Arc<Notify>,
}

impl Sources {
    /// A place for a connection from `source`, evicting the oldest past `most`.
    pub(super) fn enter(&self, source: IpAddr, most: usize) -> Unproven {
        // Each change leaves the map whole, so a poisoned lock is as good.
        let mut open = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = open.0;
        open.0 = id.wrapping_add(1);
        let queue = open.1.entry(source).or_default();
        while queue.len() >= most.max(1) {
            match queue.pop_front() {
                Some((_, oldest)) => oldest.notify_one(),
                None => break,
            }
        }
        let evicted = Arc::new(Notify::new());
        queue.push_back((id, Arc::clone(&evicted)));
        let sources = self.clone();
        Unproven {
            sources,
            source,
            id,
            evicted,
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

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
