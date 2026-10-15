//! Allocation observations are thread-local and active only around a tested seam.
//! Input creation, formatting assertions, other threads and framing are excluded.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct ObservedSystem;
#[global_allocator]
static ALLOCATOR: ObservedSystem = ObservedSystem;
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static MAXIMUM: Cell<usize> = const { Cell::new(0) };
    static TOTAL: Cell<usize> = const { Cell::new(0) };
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
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: same contract and layout as the original allocator call.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: forwarding the caller's original allocation pointer/layout.
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        // SAFETY: forwarding the caller's pointer/layout/new-size unchanged.
        unsafe { System.realloc(pointer, layout, size) }
    }
}
pub(crate) struct Scope;
impl Scope {
    pub(crate) fn start() -> Self {
        ACTIVE.with(|value| assert!(!value.replace(true), "no nested measurement"));
        MAXIMUM.with(|value| value.set(0));
        TOTAL.with(|value| value.set(0));
        Self
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
