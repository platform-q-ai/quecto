use super::{AGGREGATES, STANDING};

/// #2340: the standing's two totals are two of the report's aggregates,
/// expression and alias alike, so the budget and the receipt read the
/// values and types the report would.
#[test]
fn the_standing_is_two_of_the_reports_aggregates() {
    let aggregates: Vec<&str> = AGGREGATES.split(", ").collect();
    let standing: Vec<&str> = STANDING.split(", ").collect();
    assert_eq!(
        standing,
        [
            "coalesce(sum(tokens),0) observed_tokens",
            "coalesce(sum(unknown),0) unknown_usage_requests",
        ]
    );
    for expression in standing {
        assert!(aggregates.contains(&expression), "{expression}");
    }
}
