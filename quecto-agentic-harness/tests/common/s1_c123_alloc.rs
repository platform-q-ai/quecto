//! Allocation observations are thread-local and active only around a tested seam.
//! Input creation, formatting assertions, other threads and framing are excluded.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};

struct ObservedSystem;
#[global_allocator]
static ALLOCATOR: ObservedSystem = ObservedSystem;
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static MAXIMUM: Cell<usize> = const { Cell::new(0) };
    static TOTAL: Cell<usize> = const { Cell::new(0) };
    static LIVE: RefCell<Live> = const { RefCell::new(Live { slots: [(0, 0); 256], current: 0, peak: 0, overflow: false }) };
}
struct Live {
    slots: [(usize, usize); 256],
    current: usize,
    peak: usize,
    overflow: bool,
}
fn release(pointer: *mut u8) {
    let _ = LIVE.try_with(|state| {
        let mut state = state.borrow_mut();
        if let Some(index) = state
            .slots
            .iter()
            .position(|slot| slot.0 == pointer as usize && slot.0 != 0)
        {
            state.current -= state.slots[index].1;
            state.slots[index] = (0, 0);
        }
    });
}
fn admit(pointer: *mut u8, size: usize) {
    if let (Some(pointer), true) = (
        std::ptr::NonNull::new(pointer),
        ACTIVE.try_with(Cell::get).unwrap_or(false),
    ) {
        let _ = LIVE.try_with(|state| {
            let mut state = state.borrow_mut();
            if let Some(index) = state.slots.iter().position(|slot| slot.0 == 0) {
                state.slots[index] = (pointer.as_ptr() as usize, size);
                state.current += size;
                state.peak = state.peak.max(state.current);
            } else {
                state.overflow = true;
            }
        });
    }
}
fn replaced(pointer: *mut u8, replacement: *mut u8, size: usize) {
    if let Some(replacement) = std::ptr::NonNull::new(replacement) {
        release(pointer);
        admit(replacement.as_ptr(), size);
    }
}
fn record(size: usize) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _ = MAXIMUM.try_with(|value| value.set(value.get().max(size)));
        let _ = TOTAL.try_with(|value| value.set(value.get().saturating_add(size)));
    }
}
// SAFETY: all allocation/deallocation operations forward the original pointer,
// layout and size to System. Observation touches only nonallocating TLS Cells.
unsafe impl GlobalAlloc for ObservedSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: the caller supplies the allocator contract's valid layout.
        let pointer = unsafe { System.alloc(layout) };
        admit(pointer, layout.size());
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: same contract and layout as the original allocator call.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        admit(pointer, layout.size());
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        release(pointer);
        // SAFETY: forwarding the caller's original allocation pointer/layout.
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        // SAFETY: forwarding the caller's pointer/layout/new-size unchanged.
        let replacement = unsafe { System.realloc(pointer, layout, size) };
        replaced(pointer, replacement, size);
        replacement
    }
}
pub(crate) struct Scope;
impl Scope {
    pub(crate) fn start() -> Self {
        ACTIVE.with(|value| assert!(!value.get(), "no nested measurement"));
        LIVE.with(|state| {
            *state.borrow_mut() = Live {
                slots: [(0, 0); 256],
                current: 0,
                peak: 0,
                overflow: false,
            }
        });
        ACTIVE.with(|value| value.set(true));
        MAXIMUM.with(|value| value.set(0));
        TOTAL.with(|value| value.set(0));
        Self
    }
    pub(crate) fn finish_live(self) -> usize {
        ACTIVE.with(|value| value.set(false));
        LIVE.with(|state| {
            let state = state.borrow();
            assert!(!state.overflow, "scope pointer identity table overflow");
            state.peak
        })
    }
    pub(crate) fn finish(self) -> (usize, usize) {
        ACTIVE.with(|value| value.set(false));
        (MAXIMUM.with(Cell::get), TOTAL.with(Cell::get))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|value| value.set(false));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[test]
    fn remediation_live_allocator_tracks_identity_release_and_peak_not_total() {
        let outside = Vec::<u8>::with_capacity(5000);
        let scope = Scope::start();
        drop(outside);
        for _ in 0..20 {
            let owned = Vec::<u8>::with_capacity(100);
            std::hint::black_box(&owned);
        }
        assert_eq!(scope.finish_live(), 100);
    }
    #[test]
    fn remediation_live_allocator_realloc_success_failure_and_preexisting_identity() {
        let scope = Scope::start();
        let first = std::ptr::without_provenance_mut(1);
        let second = std::ptr::without_provenance_mut(2);
        admit(first, 100);
        replaced(first, std::ptr::null_mut(), 200);
        LIVE.with(|state| assert_eq!(state.borrow().current, 100));
        replaced(first, second, 200);
        LIVE.with(|state| assert_eq!(state.borrow().current, 200));
        release(first);
        LIVE.with(|state| assert_eq!(state.borrow().current, 200));
        release(second);
        LIVE.with(|state| assert_eq!(state.borrow().current, 0));
        replaced(first, second, 50);
        LIVE.with(|state| assert_eq!(state.borrow().current, 50));
        release(second);
        assert_eq!(scope.finish_live(), 200);
    }
}
