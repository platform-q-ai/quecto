use super::*;

#[test]
fn text_constructs_a_zero_usage_result() {
    let r = AgentResult::text("hello");
    assert_eq!(r.response, "hello");
    assert_eq!(r.tool_iterations, 0);
    assert!(!r.iteration_limit_reached);
    assert_eq!(r.turn_tokens(), 0);
    assert!(!r.has_usage(), "a fresh text result reports no usage");
}

#[test]
fn turn_tokens_sums_input_and_output_saturating() {
    let mut r = AgentResult::text("");
    r.input_tokens = u32::MAX;
    r.output_tokens = 10;
    assert_eq!(r.turn_tokens(), u32::MAX, "saturates, never overflows");

    let mut r = AgentResult::text("");
    r.input_tokens = 5;
    r.output_tokens = 7;
    assert_eq!(r.turn_tokens(), 12);
    assert!(r.has_usage());
}

#[test]
fn has_usage_true_for_any_billed_field() {
    let setters: [fn(&mut AgentResult); 5] = [
        |r| r.billed_input_tokens = 1,
        |r| r.billed_output_tokens = 1,
        |r| r.cache_read_tokens = 1,
        |r| r.cache_write_tokens = 1,
        |r| r.cost_micro_usd = 1,
    ];
    for set in setters {
        let mut r = AgentResult::text("");
        assert!(!r.has_usage());
        set(&mut r);
        assert!(r.has_usage());
    }
}
