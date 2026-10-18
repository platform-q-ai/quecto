use super::*;

const TOOLS: usize = 500;
const STUB: usize = 50;

#[derive(Debug, Clone)]
enum Row {
    System(usize),
    Prompt(usize),
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

    fn prompt(self, tokens: usize) -> Self {
        self.push(Row::Prompt(tokens))
    }

    /// A user-role message the harness added.
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
                Row::Prompt(tokens) => (PlanRole::Prompt, *tokens),
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

fn size(tool_tokens: usize, message_tokens: usize) -> RequestSize {
    RequestSize {
        tool_tokens,
        message_tokens,
    }
}

/// The messages' tokens alone.
fn message_tokens(messages: &[PlanMessage<'_>]) -> usize {
    messages.iter().map(|m| m.tokens).sum()
}

fn trigger(high: usize, low: usize) -> CutTrigger {
    CutTrigger {
        marks: Watermark::new(high, low).expect("valid marks"),
        ceiling: usize::MAX,
        after_last_cut: None,
    }
}

fn input<'a>(messages: &'a [PlanMessage<'a>], high: usize, low: usize) -> CutInput<'a> {
    CutInput {
        messages,
        tool_tokens: TOOLS,
        stub_tokens: STUB,
        trigger: trigger(high, low),
    }
}

/// `input` with `after_last_cut` and `ceiling` set.
fn later<'a>(
    messages: &'a [PlanMessage<'a>],
    (high, low): (usize, usize),
    ceiling: usize,
    after_last_cut: Option<usize>,
) -> CutInput<'a> {
    let base = input(messages, high, low);
    CutInput {
        trigger: CutTrigger {
            ceiling,
            after_last_cut,
            ..base.trigger
        },
        ..base
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

/// Every index is kept or archived, once; no call is split from its result
/// (a result answers the latest call before it with its id).
fn assert_whole(messages: &[PlanMessage<'_>], plan: &CutPlan) {
    let mut seen: Vec<usize> = plan.kept().to_vec();
    seen.extend(plan.archived().iter().flat_map(Clone::clone));
    seen.sort_unstable();
    assert_eq!(seen, (0..messages.len()).collect::<Vec<_>>());
    for (r, message) in messages.iter().enumerate() {
        if let PlanRole::ToolResult { call } = message.role {
            let made = |m: &PlanMessage<'_>| matches!(&m.role, PlanRole::Assistant { calls } if calls.contains(&call));
            if let Some(c) = messages[..r].iter().rposition(made) {
                assert_eq!(
                    plan.kept().contains(&c),
                    plan.kept().contains(&r),
                    "call {c} split from result {r}: {plan:?}"
                );
            }
        }
    }
}

#[test]
fn below_the_high_mark_there_is_no_plan() {
    let script = Script::default().system(100).prompt(200).exchanges(20);
    let messages = script.messages();
    let at = total(&messages);
    assert_eq!(
        plan_cut(&input(&messages, at + 1, 1_000)),
        Err(NoCut::NotDue)
    );
    assert!(plan_cut(&input(&messages, at, 1_000)).is_ok(), "at H");
}

#[test]
fn the_trigger_alone_says_when_a_cut_is_due() {
    let due = trigger(10_000, 4_000);
    assert!(!due.is_due(size(500, 9_499)));
    assert!(due.is_due(size(500, 9_500)), "tools count toward H");
    // The storm gap is 3,000 message tokens over a 38,000 baseline.
    let held = CutTrigger {
        after_last_cut: Some(38_000),
        ceiling: 40_000,
        ..due
    };
    assert!(
        !held.is_due(size(1_000, 39_000)),
        "exactly the ceiling is sendable"
    );
    assert!(
        held.is_due(size(1_001, 39_000)),
        "over the ceiling the gap gives way"
    );
    assert!(held.is_due(size(0, 41_000)), "grown by the gap");
    assert!(
        held.reset().is_due(size(0, 12_000)),
        "a reset drops the baseline"
    );
}

#[test]
fn marks_too_close_to_save_enough_are_refused() {
    assert_eq!(
        Watermark::new(100_000, 95_000),
        Err(InvalidMarks::GapUnderMinSaving {
            high: 100_000,
            low: 95_000,
            min_gap: 10_000
        })
    );
    assert!(
        Watermark::new(100_000, 90_000).is_ok(),
        "a gap of exactly H/10"
    );
    assert!(Watermark::new(100_000, 90_001).is_err());
    assert_eq!(Watermark::new(100, 0), Err(InvalidMarks::LowNotPositive));
    assert_eq!(
        Watermark::new(100, 100),
        Err(InvalidMarks::LowNotBelowHigh {
            high: 100,
            low: 100
        })
    );
    let refused = Watermark::new(100_000, 95_000).expect_err("too close");
    assert!(refused.to_string().contains("at most 90000"), "{refused}");
}

#[test]
fn at_the_high_mark_the_head_the_stub_and_the_newest_exchanges_fit_within_low() {
    let script = Script::default().system(100).prompt(200).exchanges(40);
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
    assert_eq!(plan.fill(), Fill::WithinLow);
    assert_whole(&messages, &plan);
    let exact = plan_cut(&input(&messages, total(&messages), 9_850));
    assert_eq!(exact, Ok(plan), "a tail of exactly L fits");
}

#[test]
fn the_latest_prompt_stays_pinned_in_a_long_single_prompt_turn() {
    let script = Script::default()
        .system(100)
        .prompt(200)
        .exchanges(5)
        .prompt(150)
        .exchanges(60);
    let messages = script.messages();
    let prompt = 12;
    assert_eq!(messages[prompt].role, PlanRole::Prompt);
    let plan = plan_cut(&input(&messages, total(&messages), 10_000)).expect("at H");

    assert_eq!(plan.stub_slot(), 3, "system, brief, latest prompt");
    assert_eq!(plan.kept()[..3], [0, 1, prompt]);
    let tail_start = plan.kept()[3];
    assert_eq!(spans(&plan), [(2, prompt), (prompt + 1, tail_start)]);
    assert!(plan.projected_tokens() <= 10_000);
    assert_whole(&messages, &plan);
}

#[test]
fn a_harness_user_message_does_not_unpin_the_running_prompt() {
    let script = Script::default()
        .system(100)
        .prompt(200)
        .exchanges(5)
        .prompt(150)
        .exchanges(30)
        .user(20)
        .exchanges(30);
    let messages = script.messages();
    let (prompt, feedback) = (12, 73);
    assert_eq!(messages[feedback].role, PlanRole::User);
    let plan = plan_cut(&input(&messages, total(&messages), 10_000)).expect("at H");

    assert_eq!(plan.kept()[..plan.stub_slot()], [0, 1, prompt]);
    assert!(!plan.kept().contains(&feedback), "feedback is archivable");
    assert_whole(&messages, &plan);
}

#[test]
fn an_exchange_over_low_is_kept_whole() {
    let script = Script::default()
        .system(100)
        .prompt(200)
        .exchanges(10)
        .parallel(&[5_000, 5_000, 5_000]);
    let messages = script.messages();
    let low = 4_000;
    let plan = plan_cut(&input(&messages, total(&messages), low)).expect("at H");

    let newest = messages.len() - 4;
    let mut kept = vec![0, 1];
    kept.extend(newest..messages.len());
    assert_eq!(plan.kept(), kept, "the call with all three results");
    assert_eq!(plan.fill(), Fill::NewestExchangeOverLow);
    assert_eq!(plan.projected_tokens(), TOOLS + 300 + STUB + 15_010);
    assert_whole(&messages, &plan);
}

#[test]
fn a_head_over_low_is_told_apart_from_an_exchange_over_low() {
    let script = Script::default().system(6_000).prompt(200).exchanges(10);
    let messages = script.messages();
    let plan = plan_cut(&input(&messages, total(&messages), 4_000)).expect("at H");

    assert_eq!(plan.fill(), Fill::HeadOverLow, "{plan:?}");
    assert_eq!(plan.kept(), [0, 1, messages.len() - 2, messages.len() - 1]);
}

#[test]
fn a_ceiling_below_high_lowers_both_marks_proportionally() {
    let marks = Watermark::new(256_000, 70_000).expect("valid marks");
    let scaled = Watermark::new(128_000, 35_000).expect("valid marks");
    assert_eq!(marks.under_ceiling(128_000), scaled);
    assert_eq!(marks.under_ceiling(300_000), marks, "a high ceiling");

    let script = Script::default().system(100).prompt(200).exchanges(129);
    let messages = script.messages();
    let at_ceiling = |ceiling| CutInput {
        trigger: CutTrigger {
            marks,
            ceiling,
            after_last_cut: None,
        },
        ..input(&messages, 2, 1)
    };
    assert_eq!(
        plan_cut(&at_ceiling(total(&messages) + 1)),
        Err(NoCut::NotDue)
    );
    let plan = plan_cut(&at_ceiling(128_000)).expect("over the ceiling's H");
    assert!(plan.projected_tokens() <= 35_000, "{plan:?}");
    assert!(plan.projected_tokens() + 1_000 > 35_000, "{plan:?}");
}

#[test]
fn a_tiny_ceiling_keeps_a_positive_low_mark_and_still_cuts() {
    let marks = Watermark::new(256_000, 70_000).expect("valid marks");
    assert_eq!(marks.under_ceiling(3), Watermark::new(3, 1).expect("valid"));
    let floor = Watermark::new(2, 1).expect("valid");
    assert_eq!(marks.under_ceiling(1), floor);
    assert_eq!(marks.under_ceiling(0), floor);

    let script = Script::default().system(10).prompt(10).exchanges(3);
    let messages = script.messages();
    let plan =
        plan_cut(&later(&messages, (256_000, 70_000), 3, None)).expect("far over the ceiling");
    assert_eq!(plan.fill(), Fill::HeadOverLow);
    assert_whole(&messages, &plan);
}

#[test]
fn no_cut_storm_after_a_cut_over_low() {
    let script = Script::default()
        .system(100)
        .prompt(200)
        .exchanges(10)
        .parallel(&[100_000]);
    let messages = script.messages();
    let marks = (50_000, 20_000);
    let first = plan_cut(&input(&messages, marks.0, marks.1)).expect("at H");
    assert_eq!(first.fill(), Fill::NewestExchangeOverLow);
    assert!(
        first.projected_tokens() > marks.0,
        "one exchange keeps it over H"
    );
    let after = trigger(marks.0, marks.1).after_cut(&first);
    assert_eq!(
        after.after_last_cut,
        Some(first.projected_tokens() - TOOLS),
        "the baseline counts messages only"
    );

    let grown = |count| script.after_cut(&first).exchanges(count);
    let gap = (marks.0 - marks.1) / 2;
    let short = grown(gap / 1_000 - 1);
    let messages = short.messages();
    assert!(total(&messages) >= marks.0);
    let held = CutInput {
        trigger: after,
        ..input(&messages, marks.0, marks.1)
    };
    assert_eq!(
        plan_cut(&held),
        Err(NoCut::NotDue),
        "grown by under (H-L)/2"
    );

    let enough = grown(gap / 1_000);
    let messages = enough.messages();
    let due = CutInput {
        trigger: after,
        ..input(&messages, marks.0, marks.1)
    };
    let plan = plan_cut(&due).expect("grown by (H-L)/2");
    assert_whole(&messages, &plan);
}

#[test]
fn the_storm_guard_gives_way_at_the_ceiling() {
    // An earlier over-L cut left 180k; the request is now over the ceiling.
    let script = Script::default()
        .system(500)
        .prompt(1_000)
        .assistant(100_000)
        .parallel(&[168_990]);
    let messages = script.messages();
    let total = total(&messages);
    assert_eq!(total, 271_000);
    let held = later(&messages, (256_000, 70_000), total, Some(180_000));
    assert_eq!(
        plan_cut(&held),
        Err(NoCut::NotDue),
        "at exactly the ceiling the guard holds"
    );
    let over = later(&messages, (256_000, 70_000), 270_000, Some(180_000));
    let plan = plan_cut(&over).expect("over the ceiling a cut is due");
    assert_whole(&messages, &plan);
}

#[test]
fn the_storm_gap_uses_the_ceiling_scaled_marks() {
    // H 100k, L 40k under a ceiling of 50,800: H 50,800, L 20,320, so the
    // gap is 15,240 (unscaled it would be 30,000).
    let script = Script::default().system(100).prompt(200).exchanges(50);
    let messages = script.messages();
    let ceiling = total(&messages);
    let grown = message_tokens(&messages);
    let at = |after| later(&messages, (100_000, 40_000), ceiling, Some(after));
    assert_eq!(at(0).trigger.effective_marks().high(), ceiling);
    assert_eq!(plan_cut(&at(grown - 15_239)), Err(NoCut::NotDue));
    let plan = plan_cut(&at(grown - 15_240)).expect("grown by the scaled gap");
    assert!(plan.projected_tokens() <= at(0).trigger.effective_marks().low());
}

#[test]
fn only_a_reset_drops_the_baseline() {
    // A baseline from a larger conversation, since cleared or rewound.
    let script = Script::default().system(10).prompt(10).exchanges(60);
    let messages = script.messages();
    let before = later(&messages, (50_000, 20_000), usize::MAX, Some(100_000));
    assert_eq!(plan_cut(&before), Err(NoCut::NotDue), "the baseline holds");
    let reset = CutInput {
        trigger: before.trigger.reset(),
        ..before
    };
    let plan = plan_cut(&reset).expect("after the reset");
    assert_whole(&messages, &plan);
}

#[test]
fn a_tool_set_change_neither_hides_nor_fakes_growth() {
    // An over-L cut kept a 240k result; the model then made one small call.
    let script = Script::default()
        .system(5_000)
        .prompt(2_000)
        .parallel(&[60_000])
        .parallel(&[240_000]);
    let messages = script.messages();
    let tools = 20_000;
    let first = CutInput {
        tool_tokens: tools,
        ..input(&messages, 256_000, 70_000)
    };
    let first = plan_cut(&first).expect("first cut");
    let after = trigger(256_000, 70_000).after_cut(&first);
    let next = script.after_cut(&first).parallel(&[1_000]);
    let messages = next.messages();
    let with_tools = |tool_tokens| CutInput {
        tool_tokens,
        trigger: after,
        ..input(&messages, 256_000, 70_000)
    };
    assert!(
        message_tokens(&messages) + tools - 5_000 >= 256_000,
        "over H"
    );
    assert_eq!(plan_cut(&with_tools(tools)), Err(NoCut::NotDue));
    assert_eq!(
        plan_cut(&with_tools(tools - 5_000)),
        Err(NoCut::NotDue),
        "fewer tools are no reset"
    );
    assert_eq!(
        plan_cut(&with_tools(tools + 93_000)),
        Err(NoCut::NotDue),
        "more tools are no growth"
    );
}

#[test]
fn a_cut_down_to_low_does_not_hold_the_next_one_back() {
    let script = Script::default().system(100).prompt(200).exchanges(50);
    let messages = script.messages();
    let next = later(
        &messages,
        (total(&messages), 10_000),
        usize::MAX,
        Some(9_000),
    );
    assert!(plan_cut(&next).is_ok(), "H is reached again");
}

#[test]
fn a_cut_must_save_a_tenth_of_high() {
    // The newest exchange is over L; the only other archivable message is
    // the assistant reply of `reply` tokens, so the cut saves `reply - STUB`.
    let with_reply = |reply| {
        Script::default()
            .system(100)
            .prompt(200)
            .assistant(reply)
            .parallel(&[20_000])
    };
    let (high, low) = (10_000, 5_000);
    let short = with_reply(1_049);
    let messages = short.messages();
    assert_eq!(
        plan_cut(&input(&messages, high, low)),
        Err(NoCut::SavingTooSmall {
            saving: 999,
            needed: 1_000
        })
    );
    let enough = with_reply(1_050);
    let messages = enough.messages();
    let plan = plan_cut(&input(&messages, high, low)).expect("saves 1,000");
    assert_eq!(total(&messages) - plan.projected_tokens(), high / 10);

    // A cut that grows the request is never made.
    let tiny = with_reply(1);
    let messages = tiny.messages();
    assert_eq!(
        plan_cut(&input(&messages, high, low)),
        Err(NoCut::SavingTooSmall {
            saving: 0,
            needed: 1_000
        }),
        "it would grow the request"
    );
    // Over the ceiling any real saving is worth a cut.
    let small = with_reply(60);
    let messages = small.messages();
    let full = later(&messages, (high, low), total(&messages) - 1, None);
    let plan = plan_cut(&full).expect("over the ceiling");
    assert_eq!(total(&messages) - plan.projected_tokens(), 10);
}

#[test]
fn a_second_cut_chains_the_previous_stub_into_the_archive() {
    let script = Script::default().system(100).prompt(200).exchanges(40);
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
        .prompt(200)
        .push(Row::Stub(STUB))
        .parallel(&[60_000]);
    let messages = script.messages();
    assert_eq!(
        plan_cut(&input(&messages, 10_000, 5_000)),
        Err(NoCut::NoBoundary)
    );

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
        .prompt(200)
        .exchanges(7)
        .prompt(40)
        .exchanges(30);
    let messages = script.messages();
    let request = input(&messages, total(&messages), 8_000);
    let plan = plan_cut(&request).expect("at H");
    assert_eq!(plan_cut(&request), Ok(plan));
}

#[test]
fn each_reason_for_no_cut_is_told() {
    let script = Script::default().system(100).prompt(200).exchanges(5);
    let messages = script.messages();
    let below = input(&messages, total(&messages) + 1, 1_000);
    assert_eq!(plan_cut(&below), Err(NoCut::NotDue));

    let no_user = Script::default().system(100).exchanges(5);
    let messages = no_user.messages();
    let at = input(&messages, total(&messages), 1_000);
    assert_eq!(plan_cut(&at), Err(NoCut::NoUserMessage));

    let all_pinned = Script::default().system(100).prompt(20_000).prompt(300);
    let messages = all_pinned.messages();
    let at = input(&messages, total(&messages), 1_000);
    assert_eq!(plan_cut(&at), Err(NoCut::NothingArchivable));
}
