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
    assert_eq!(plan_cut(&input(&messages, at + 1, 1_000)), None);
    assert!(plan_cut(&input(&messages, at, 1_000)).is_some(), "at H");
}

#[test]
fn the_trigger_alone_says_when_a_cut_is_due() {
    let due = trigger(10_000, 4_000);
    assert!(!due.is_due(9_999));
    assert!(due.is_due(10_000));
    let held = CutTrigger {
        after_last_cut: Some(30_000),
        ceiling: 40_000,
        ..due
    };
    assert!(
        !held.is_due(32_999),
        "inside the storm gap, under the ceiling"
    );
    assert!(held.is_due(40_000), "at the ceiling the gap gives way");
    let stale = CutTrigger {
        after_last_cut: Some(30_000),
        ..due
    };
    assert!(stale.is_due(12_000), "a baseline above the total is stale");
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
    assert_eq!(exact, Some(plan), "a tail of exactly L fits");
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
    assert_eq!(plan_cut(&at_ceiling(total(&messages) + 1)), None);
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
    let after = first.projected_tokens();
    assert!(after > marks.0, "a single exchange keeps it above H");

    let grown = |count| script.after_cut(&first).exchanges(count);
    let gap = (marks.0 - marks.1) / 2;
    let short = grown(gap / 1_000 - 1);
    let messages = short.messages();
    assert!(total(&messages) >= marks.0);
    let held = later(&messages, marks, usize::MAX, Some(after));
    assert_eq!(plan_cut(&held), None, "grown by less than (H-L)/2");

    let enough = grown(gap / 1_000);
    let messages = enough.messages();
    let due = later(&messages, marks, usize::MAX, Some(after));
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
    let held = later(&messages, (256_000, 70_000), total + 1, Some(180_000));
    assert_eq!(plan_cut(&held), None, "under the ceiling the guard holds");
    let over = later(&messages, (256_000, 70_000), 270_000, Some(180_000));
    let plan = plan_cut(&over).expect("over the ceiling a cut is due");
    assert_whole(&messages, &plan);
}

#[test]
fn under_a_ceiling_below_high_the_effective_marks_rule_the_guard() {
    // Under a ceiling below H the effective H is the ceiling, so reaching it
    // also gives the storm guard way: the scaled gap never holds a cut.
    let script = Script::default().system(100).prompt(200).exchanges(50);
    let messages = script.messages();
    let ceiling = total(&messages);
    let request = later(&messages, (100_000, 40_000), ceiling, Some(ceiling - 1_000));
    assert_eq!(request.trigger.effective_marks().high(), ceiling);
    let plan = plan_cut(&request).expect("at the effective H");
    assert!(plan.projected_tokens() <= request.trigger.effective_marks().low());
}

#[test]
fn a_stale_baseline_does_not_block_cuts() {
    // A baseline from a larger, since-cleared conversation.
    let script = Script::default().system(10).prompt(10).exchanges(60);
    let messages = script.messages();
    let stale = later(&messages, (50_000, 20_000), usize::MAX, Some(100_000));
    assert!(total(&messages) < 100_000);
    let plan = plan_cut(&stale).expect("the stale baseline is reset");
    assert_whole(&messages, &plan);
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
    assert!(plan_cut(&next).is_some(), "H is reached again");
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
    assert_eq!(plan_cut(&input(&messages, high, low)), None, "saves 999");
    let enough = with_reply(1_050);
    let messages = enough.messages();
    let plan = plan_cut(&input(&messages, high, low)).expect("saves 1,000");
    assert_eq!(total(&messages) - plan.projected_tokens(), high / 10);

    // A cut that grows the request is never made.
    let tiny = with_reply(1);
    let messages = tiny.messages();
    assert_eq!(plan_cut(&input(&messages, high, low)), None, "grows it");
    // At the ceiling any real saving is worth a cut.
    let small = with_reply(60);
    let messages = small.messages();
    let full = later(&messages, (high, low), total(&messages), None);
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
        .prompt(200)
        .exchanges(7)
        .prompt(40)
        .exchanges(30);
    let messages = script.messages();
    let request = input(&messages, total(&messages), 8_000);
    let plan = plan_cut(&request).expect("at H");
    assert_eq!(plan_cut(&request), Some(plan));
}
