use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use super::*;

struct CountingAllocator;

#[cfg(not(coverage))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AllocationMetrics {
    count: usize,
    bytes: usize,
}

thread_local! {
    static TRACK_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
    static ALLOCATION_COUNT: Cell<usize> = const { Cell::new(0) };
    static ALLOCATION_BYTES: Cell<usize> = const { Cell::new(0) };
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK_ALLOCATIONS.with(Cell::get) {
            ALLOCATION_COUNT.with(|count| count.set(count.get() + 1));
            ALLOCATION_BYTES.with(|bytes| bytes.set(bytes.get() + layout.size()));
        }
        // SAFETY: Delegates directly to the system allocator with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Delegates directly to the system allocator with the caller's pointer/layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[test]
fn exact_match() {
    let m = fuzzy_match("model", "model");
    assert!(m.matches);
}

#[test]
fn prefix_match() {
    let m = fuzzy_match("mod", "model");
    assert!(m.matches);
}

#[test]
fn substring_match() {
    let m = fuzzy_match("del", "model");
    assert!(m.matches);
}

#[test]
fn no_match() {
    let m = fuzzy_match("xyz", "model");
    assert!(!m.matches);
}

#[test]
fn case_insensitive() {
    let m = fuzzy_match("MODEL", "model");
    assert!(m.matches);
}

#[test]
fn empty_query_matches() {
    let m = fuzzy_match("", "anything");
    assert!(m.matches);
    assert_eq!(m.score, 0.0);
}

#[test]
fn query_longer_than_text_fails() {
    let m = fuzzy_match("longquery", "short");
    assert!(!m.matches);
}

#[test]
fn word_boundary_scores_better() {
    let exact = fuzzy_match("cl", "claude");
    let mid = fuzzy_match("cl", "include");
    assert!(exact.matches);
    assert!(mid.matches);
    // Word boundary (start of word) should score better (lower).
    assert!(
        exact.score < mid.score,
        "boundary match should score better: {} vs {}",
        exact.score,
        mid.score
    );
}

#[test]
fn filter_returns_all_on_empty_query() {
    let items = vec!["a", "b", "c"];
    let results = fuzzy_filter(&items, "", |s| s);
    assert_eq!(results.len(), 3);
}

#[test]
fn filter_by_prefix() {
    let items = vec!["settings", "model", "clear", "quit"];
    let results = fuzzy_filter(&items, "set", |s| s);
    assert_eq!(results.len(), 1);
    assert_eq!(*results[0], "settings");
}

#[test]
fn filter_multi_token() {
    let items = vec!["claude-sonnet-4", "gpt-4o", "claude-opus-4"];
    let results = fuzzy_filter(&items, "claude opus", |s| s);
    assert_eq!(results.len(), 1);
    assert_eq!(*results[0], "claude-opus-4");
}

#[test]
fn filter_sorts_by_score() {
    let items = vec!["abcmodel", "model", "modeler"];
    let results = fuzzy_filter(&items, "model", |s| s);
    assert_eq!(results, vec![&"model", &"modeler", &"abcmodel"]);
}

#[test]
fn match_handles_case_folding_that_expands_to_multiple_chars() {
    let m = fuzzy_match("i̇st", "İstanbul.rs");
    assert!(m.matches);
}

#[test]
fn filter_handles_cjk_candidates() {
    let items = vec!["src/東京.rs", "src/京都.rs"];
    let results = fuzzy_filter(&items, "東京", |s| s);
    assert_eq!(results, vec![&"src/東京.rs"]);
}

#[cfg(not(coverage))]
#[test]
fn limited_filter_allocations_stay_constant_with_candidate_count() {
    for query in ["file", "missing", "file module"] {
        let small = workspace_files(100);
        let capped = workspace_files(5_000);

        let small_allocations = allocations_during(|| {
            let _ = fuzzy_filter_limited(&small, query, 32, |s| s.as_str());
        });
        let capped_allocations = allocations_during(|| {
            let _ = fuzzy_filter_limited(&capped, query, 32, |s| s.as_str());
        });

        assert_eq!(
            capped_allocations, small_allocations,
            "limited filtering should not add allocations as candidates grow for {query:?}: small={small_allocations:?}, capped={capped_allocations:?}"
        );
        assert!(
            capped_allocations.count <= 5,
            "limited filtering should only allocate fixed query/result bookkeeping for {query:?}: capped={capped_allocations:?}"
        );
        assert!(
            capped_allocations.bytes <= 1_024,
            "limited filtering should keep fixed bookkeeping bytes bounded for {query:?}: capped={capped_allocations:?}"
        );
    }
}

#[test]
fn limited_filter_pins_order_for_boundaries_and_case() {
    let items = vec![
        "abcmodel.rs",
        "src/Model.rs",
        "src/foo-model.rs",
        "docs/model.rs",
        "x/MODELER.rs",
    ];
    let limited = fuzzy_filter_limited(&items, "MODEL", 4, |s| s);
    assert_eq!(
        limited,
        vec![
            &"x/MODELER.rs",
            &"src/Model.rs",
            &"docs/model.rs",
            &"src/foo-model.rs",
        ]
    );
}

#[test]
fn limited_filter_pins_order_with_expanding_case_fold() {
    let items = vec![
        "src/xİstanbul.rs",
        "src/İstanbul.rs",
        "docs/istanbul.rs",
        "src/foo-İstanbul.rs",
    ];
    let limited = fuzzy_filter_limited(&items, "i̇st", 3, |s| s);
    assert_eq!(
        limited,
        vec![
            &"src/İstanbul.rs",
            &"src/foo-İstanbul.rs",
            &"src/xİstanbul.rs",
        ]
    );
}

#[test]
fn limited_filter_pins_order_with_multiple_cjk_matches() {
    let items = vec!["src/x東京.rs", "src/東京.rs", "docs/東京_notes.md"];
    let limited = fuzzy_filter_limited(&items, "東京", 2, |s| s);
    assert_eq!(limited, vec![&"src/東京.rs", &"docs/東京_notes.md"]);
}

#[cfg(not(coverage))]
fn workspace_files(file_count: usize) -> Vec<String> {
    (0..file_count)
        .map(|i| format!("src/module_{i:04}/file_{i:04}.rs"))
        .collect()
}

#[cfg(not(coverage))]
fn allocations_during(work: impl FnOnce()) -> AllocationMetrics {
    ALLOCATION_COUNT.with(|count| count.set(0));
    ALLOCATION_BYTES.with(|bytes| bytes.set(0));
    TRACK_ALLOCATIONS.with(|tracking| tracking.set(true));
    work();
    TRACK_ALLOCATIONS.with(|tracking| tracking.set(false));
    AllocationMetrics {
        count: ALLOCATION_COUNT.with(Cell::get),
        bytes: ALLOCATION_BYTES.with(Cell::get),
    }
}
