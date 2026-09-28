use super::{LineLimits, MIN_LINE_COST, StdoutLine};

#[test]
fn limits_are_valid_only_when_a_line_fits_the_budget() {
    let valid = |line_cap, buffer_bytes| {
        LineLimits {
            line_cap,
            buffer_bytes,
        }
        .valid()
    };
    assert!(valid(1024, 4096));
    assert!(valid(MIN_LINE_COST, MIN_LINE_COST));
    assert!(!valid(0, 4096), "a line holds at least one byte");
    assert!(
        !valid(8192, 4096),
        "a line longer than the budget never fits"
    );
    assert!(!valid(16, 128), "the budget holds at least one entry");
    assert!(
        !valid(1024, usize::try_from(u64::from(u32::MAX) + 1).unwrap()),
        "the budget is one acquisition"
    );
}

#[test]
fn each_line_costs_its_bytes_at_least_the_minimum_at_most_the_budget() {
    let limits = LineLimits {
        line_cap: 4096,
        buffer_bytes: 4096,
    };
    let cost = |line: StdoutLine| line.cost(limits) as usize;
    assert_eq!(cost(StdoutLine::OverCap { bytes: 1 << 30 }), MIN_LINE_COST);
    assert_eq!(cost(StdoutLine::Line(Vec::new())), MIN_LINE_COST);
    assert_eq!(cost(StdoutLine::Line(Vec::with_capacity(2000))), 2000);
    assert_eq!(cost(StdoutLine::Line(Vec::with_capacity(9000))), 4096);
}
