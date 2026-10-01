use super::*;

const TOOLS: usize = 500;
const STUB: usize = 50;

#[derive(Debug, Clone)]
enum Row {
    System(usize),
    User(usize),
    Stub(usize),
    Assistant(usize, Vec<String>),
    Result(usize, String),
}

/// A conversation under construction; it owns the call ids the plan
/// messages borrow.
#[derive(Debug, Clone, Default)]
struct Script {
    rows: Vec<Row>,
    next_call: usize,
}

impl Script {
    fn push(mut self, row: Row) -> Self {
        self.rows.push(row);
        self
    }

    fn system(self, tokens: usize) -> Self {
        self.push(Row::System(tokens))
    }

    fn user(self, tokens: usize) -> Self {
        self.push(Row::User(tokens))
    }

    fn assistant(self, tokens: usize) -> Self {
        self.push(Row::Assistant(tokens, Vec::new()))
    }

    /// One assistant message making a call per result, then the results.
    fn parallel(mut self, results: &[usize]) -> Self {
        let ids: Vec<String> = (0..results.len())
            .map(|n| format!("call-{}", self.next_call + n))
            .collect();
        self.next_call += results.len();
        self.rows.push(Row::Assistant(10, ids.clone()));
        for (id, tokens) in ids.into_iter().zip(results) {
            self.rows.push(Row::Result(*tokens, id));
        }
        self
    }

    /// `count` exchanges of 1,000 tokens: a 10-token call, its result.
    fn exchanges(self, count: usize) -> Self {
        (0..count).fold(self, |script, _| script.parallel(&[990]))
    }

    fn messages(&self) -> Vec<PlanMessage<'_>> {
        self.rows
            .iter()
            .map(|row| match row {
                Row::System(tokens) => (PlanRole::System, *tokens),
                Row::User(tokens) => (PlanRole::User, *tokens),
                Row::Stub(tokens) => (PlanRole::ArchiveStub, *tokens),
                Row::Assistant(tokens, ids) => (
                    PlanRole::Assistant {
                        calls: ids.iter().map(String::as_str).collect(),
                    },
                    *tokens,
                ),
                Row::Result(tokens, id) => (PlanRole::ToolResult { call: id.as_str() }, *tokens),
            })
            .map(|(role, tokens)| PlanMessage { role, tokens })
            .collect()
    }

    /// The conversation once `plan` is made: the kept rows with the stub.
    fn after_cut(&self, plan: &CutPlan) -> Self {
        let mut rows: Vec<Row> = plan.kept().iter().map(|&i| self.rows[i].clone()).collect();
        rows.insert(plan.stub_slot(), Row::Stub(STUB));
        Self {
            rows,
            next_call: self.next_call,
        }
    }
}

fn total(messages: &[PlanMessage<'_>]) -> usize {
    TOOLS + messages.iter().map(|m| m.tokens).sum::<usize>()
}

fn input<'a>(messages: &'a [PlanMessage<'a>], high: usize, low: usize) -> CutInput<'a> {
    CutInput {
        messages,
        tool_tokens: TOOLS,
        stub_tokens: STUB,
        marks: Watermark::new(high, low).expect("valid marks"),
        ceiling: usize::MAX,
        after_last_cut: None,
    }
}

/// The request's tokens once `plan` is made, counted independently.
fn kept_tokens(messages: &[PlanMessage<'_>], plan: &CutPlan) -> usize {
    TOOLS
        + STUB
        + plan
            .kept()
            .iter()
            .map(|&i| messages[i].tokens)
            .sum::<usize>()
}

/// The archived ranges as `(start, end)` pairs.
fn spans(plan: &CutPlan) -> Vec<(usize, usize)> {
    plan.archived().iter().map(|r| (r.start, r.end)).collect()
}

/// Every index is kept or archived, once; no call is split from its result.
fn assert_whole(messages: &[PlanMessage<'_>], plan: &CutPlan) {
    let mut seen: Vec<usize> = plan.kept().to_vec();
    seen.extend(plan.archived().iter().flat_map(Clone::clone));
    seen.sort_unstable();
    assert_eq!(seen, (0..messages.len()).collect::<Vec<_>>());
    for (r, message) in messages.iter().enumerate() {
        if let PlanRole::ToolResult { call } = message.role {
            let c = messages[..r]
                .iter()
                .rposition(
                    |m| matches!(&m.role, PlanRole::Assistant { calls } if calls.contains(&call)),
                )
                .expect("every result has its call");
            assert_eq!(
                plan.kept().contains(&c),
                plan.kept().contains(&r),
                "call {c} split from result {r}: {plan:?}"
            );
        }
    }
}

#[test]
fn below_the_high_mark_there_is_no_plan() {
    let script = Script::default().system(100).user(200).exchanges(20);
    let messages = script.messages();
    let at = total(&messages);
    assert_eq!(plan_cut(&input(&messages, at + 1, 1_000)), None);
    assert!(plan_cut(&input(&messages, at, 1_000)).is_some(), "at H");
}

#[test]
fn at_the_high_mark_the_head_the_stub_and_the_newest_exchanges_fit_within_low() {
    let script = Script::default().system(100).user(200).exchanges(40);
    let messages = script.messages();
    let low = 10_000;
    let plan = plan_cut(&input(&messages, total(&messages), low)).expect("at H");

    let tail_start = messages.len() - 2 * 9;
    let mut kept = vec![0, 1];
    kept.extend(tail_start..messages.len());
    assert_eq!(
        plan.kept(),
        kept,
        "system, brief and the nine newest exchanges"
    );
    assert_eq!(spans(&plan), [(2, tail_start)]);
    assert_eq!(plan.stub_slot(), 2, "the stub follows the pinned head");
    assert_eq!(plan.projected_tokens(), kept_tokens(&messages, &plan));
    assert_eq!(plan.projected_tokens(), 9_850);
    assert!(
        plan.projected_tokens() + 1_000 > low,
        "one more would not fit"
    );
    assert!(!plan.over_low_mark());
    assert_whole(&messages, &plan);
    let exact = plan_cut(&input(&messages, total(&messages), 9_850));
    assert_eq!(exact, Some(plan), "a tail of exactly L fits");
}

#[test]
fn a_tool_call_is_never_split_from_its_result_whatever_the_marks() {
    let mut script = Script::default().system(100).user(200);
    for n in 0..30 {
        script = match n % 5 {
            0 => script.parallel(&[300, 40 * n + 1, 700]),
            1 => script.assistant(80),
            2 => script.user(30),
            _ => script.parallel(&[(n * 37) % 900 + 1]),
        };
    }
    let messages = script.messages();
    let full = total(&messages);
    for high in (full / 4..=full).step_by(997) {
        for low in [1, 100, 1_000, high / 2, high - 1] {
            let plan = plan_cut(&input(&messages, high, low))
                .unwrap_or_else(|| panic!("a cut at H={high} L={low}"));
            assert_whole(&messages, &plan);
            assert_eq!(plan.over_low_mark(), plan.projected_tokens() > low);
        }
    }
}

#[test]
fn the_latest_prompt_stays_pinned_in_a_long_single_prompt_turn() {
    let script = Script::default()
        .system(100)
        .user(200)
        .exchanges(5)
        .user(150)
        .exchanges(60);
    let messages = script.messages();
    let prompt = 12;
    assert_eq!(messages[prompt].role, PlanRole::User);
    let plan = plan_cut(&input(&messages, total(&messages), 10_000)).expect("at H");

    assert_eq!(plan.stub_slot(), 3, "system, brief, latest prompt");
    assert_eq!(plan.kept()[..3], [0, 1, prompt]);
    let tail_start = plan.kept()[3];
    assert_eq!(spans(&plan), [(2, prompt), (prompt + 1, tail_start)]);
    assert!(plan.projected_tokens() <= 10_000);
    assert_whole(&messages, &plan);
}

#[test]
fn an_exchange_over_low_is_kept_whole() {
    let script = Script::default()
        .system(100)
        .user(200)
        .exchanges(10)
        .parallel(&[5_000, 5_000, 5_000]);
    let messages = script.messages();
    let low = 4_000;
    let plan = plan_cut(&input(&messages, total(&messages), low)).expect("at H");

    let newest = messages.len() - 4;
    let mut kept = vec![0, 1];
    kept.extend(newest..messages.len());
    assert_eq!(plan.kept(), kept, "the call with all three results");
    assert!(plan.over_low_mark());
    assert_eq!(plan.projected_tokens(), TOOLS + 300 + STUB + 15_010);
    assert_whole(&messages, &plan);
}

#[test]
fn a_ceiling_below_high_lowers_both_marks_proportionally() {
    let marks = Watermark::new(256_000, 70_000).expect("valid marks");
    assert_eq!(
        marks.under_ceiling(128_000),
        Watermark::new(128_000, 35_000)
    );
    assert_eq!(marks.under_ceiling(300_000), Some(marks), "a high ceiling");
    assert_eq!(marks.under_ceiling(0), None);

    let script = Script::default().system(100).user(200).exchanges(129);
    let messages = script.messages();
    let at_ceiling = |ceiling| CutInput {
        ceiling,
        marks,
        ..input(&messages, 2, 1)
    };
    assert_eq!(plan_cut(&at_ceiling(total(&messages) + 1)), None);
    let plan = plan_cut(&at_ceiling(128_000)).expect("over the ceiling's H");
    assert!(plan.projected_tokens() <= 35_000, "{plan:?}");
    assert!(plan.projected_tokens() + 1_000 > 35_000, "{plan:?}");
}

#[test]
fn no_cut_storm_after_a_cut_over_low() {
    let script = Script::default()
        .system(100)
        .user(200)
        .exchanges(10)
        .parallel(&[100_000]);
    let messages = script.messages();
    let (high, low) = (50_000, 20_000);
    let first = plan_cut(&input(&messages, high, low)).expect("at H");
    assert!(first.over_low_mark());
    let after = first.projected_tokens();
    assert!(after > high, "a single exchange keeps it above H");

    let grown = |count| script.after_cut(&first).exchanges(count);
    let gap = (high - low) / 2;
    let short = grown(gap / 1_000 - 1);
    let messages = short.messages();
    assert!(total(&messages) >= high);
    let held = CutInput {
        after_last_cut: Some(after),
        ..input(&messages, high, low)
    };
    assert_eq!(plan_cut(&held), None, "grown by less than (H-L)/2");

    let enough = grown(gap / 1_000);
    let messages = enough.messages();
    let due = CutInput {
        after_last_cut: Some(after),
        ..input(&messages, high, low)
    };
    let plan = plan_cut(&due).expect("grown by (H-L)/2");
    assert_whole(&messages, &plan);
}

#[test]
fn a_cut_down_to_low_does_not_hold_the_next_one_back() {
    let script = Script::default().system(100).user(200).exchanges(50);
    let messages = script.messages();
    let next = CutInput {
        after_last_cut: Some(9_000),
        ..input(&messages, total(&messages), 10_000)
    };
    assert!(plan_cut(&next).is_some(), "H is reached again");
}

#[test]
fn a_second_cut_chains_the_previous_stub_into_the_archive() {
    let script = Script::default().system(100).user(200).exchanges(40);
    let messages = script.messages();
    let (high, low) = (total(&messages), 10_000);
    let first = plan_cut(&input(&messages, high, low)).expect("first cut");

    let cut = script.after_cut(&first).exchanges(31);
    let messages = cut.messages();
    assert_eq!(messages[2].role, PlanRole::ArchiveStub);
    let second = plan_cut(&input(&messages, high, low)).expect("second cut");
    assert_eq!(second.archived()[0].start, 2, "the old stub is archived");
    assert!(!second.kept().contains(&2));
    assert_eq!(second.stub_slot(), 2, "one new stub after the head");
    assert!(second.projected_tokens() <= low);
    assert_whole(&messages, &second);
}

#[test]
fn archiving_only_the_previous_stub_is_no_cut_but_one_more_exchange_is() {
    let script = Script::default()
        .system(100)
        .user(200)
        .push(Row::Stub(STUB))
        .parallel(&[60_000]);
    let messages = script.messages();
    assert_eq!(plan_cut(&input(&messages, 10_000, 5_000)), None);

    let script = script.parallel(&[1_000]);
    let messages = script.messages();
    let plan = plan_cut(&input(&messages, 10_000, 5_000)).expect("a real exchange to archive");
    assert_eq!(
        spans(&plan),
        [(2, 5)],
        "the stub with the exchange after it"
    );
}

#[test]
fn the_plan_is_deterministic() {
    let script = Script::default()
        .system(100)
        .user(200)
        .exchanges(7)
        .user(40)
        .exchanges(30);
    let messages = script.messages();
    let request = input(&messages, total(&messages), 8_000);
    let plan = plan_cut(&request).expect("at H");
    assert_eq!(plan_cut(&request), Some(plan));
}
