//! The plan's promises, checked from first principles over hand-built and
//! seeded random conversations: above all, that no call is split from its
//! result.
use super::*;

const STUB: usize = 5;

fn m(role: PlanRole<'static>, tokens: usize) -> PlanMessage<'static> {
    PlanMessage { role, tokens }
}

fn s(tokens: usize) -> PlanMessage<'static> {
    m(PlanRole::System, tokens)
}

fn p(tokens: usize) -> PlanMessage<'static> {
    m(PlanRole::Prompt, tokens)
}

fn u(tokens: usize) -> PlanMessage<'static> {
    m(PlanRole::User, tokens)
}

fn a(calls: &[&'static str], tokens: usize) -> PlanMessage<'static> {
    m(
        PlanRole::Assistant {
            calls: calls.to_vec(),
        },
        tokens,
    )
}

fn r(call: &'static str, tokens: usize) -> PlanMessage<'static> {
    m(PlanRole::ToolResult { call }, tokens)
}

fn bare<'a>(
    messages: &'a [PlanMessage<'a>],
    (high, low): (usize, usize),
    ceiling: usize,
    after_last_cut: Option<usize>,
) -> CutInput<'a> {
    CutInput {
        messages,
        tool_tokens: 0,
        stub_tokens: STUB,
        trigger: CutTrigger {
            marks: Watermark::new(high, low).expect("valid marks"),
            ceiling,
            after_last_cut,
        },
    }
}

fn tokens_of<'a>(messages: &[PlanMessage<'_>], indices: impl Iterator<Item = &'a usize>) -> usize {
    indices.map(|&i| messages[i].tokens).sum()
}

/// Every promise of `plan` for `input`, checked without the planner's code.
fn assert_sound(input: &CutInput<'_>, plan: &CutPlan) {
    let messages = input.messages;
    let kept = |i: usize| plan.kept().contains(&i);
    let mut seen: Vec<usize> = plan.kept().to_vec();
    seen.extend(plan.archived().iter().flat_map(Clone::clone));
    seen.sort_unstable();
    assert!(
        seen.iter().copied().eq(0..messages.len()),
        "a partition: {plan:?}"
    );
    for (result, message) in messages.iter().enumerate() {
        if let PlanRole::ToolResult { call } = message.role {
            let made = |m: &PlanMessage<'_>| matches!(&m.role, PlanRole::Assistant { calls } if calls.contains(&call));
            if let Some(call) = messages[..result].iter().rposition(made) {
                assert_eq!(
                    kept(call),
                    kept(result),
                    "{call} split from {result}: {plan:?}"
                );
            }
        }
    }
    let users = |m: &&PlanMessage<'_>| matches!(m.role, PlanRole::Prompt | PlanRole::User);
    let first = messages
        .iter()
        .position(|m| users(&m))
        .expect("a user message");
    assert!(kept(first), "the first user message is pinned: {plan:?}");
    if let Some(prompt) = messages.iter().rposition(|m| m.role == PlanRole::Prompt) {
        assert!(kept(prompt), "the latest prompt is pinned: {plan:?}");
    }
    for (i, message) in messages.iter().enumerate() {
        match message.role {
            PlanRole::System => assert!(kept(i), "system {i} pinned: {plan:?}"),
            PlanRole::ArchiveStub => assert!(!kept(i), "stub {i} archived: {plan:?}"),
            PlanRole::Prompt
            | PlanRole::User
            | PlanRole::Assistant { .. }
            | PlanRole::ToolResult { .. } => {}
        }
    }
    let tail_start = plan.kept()[plan.stub_slot()];
    assert!(matches!(
        messages[tail_start].role,
        PlanRole::Prompt | PlanRole::User | PlanRole::Assistant { .. }
    ));
    let fixed = input.tool_tokens + input.stub_tokens;
    let projected = fixed + tokens_of(messages, plan.kept().iter());
    assert_eq!(plan.projected_tokens(), projected);
    let total = input.tool_tokens + messages.iter().map(|m| m.tokens).sum::<usize>();
    assert!(projected < total, "a cut shrinks the request: {plan:?}");
    let low = input.trigger.effective_marks().low();
    let head = fixed + tokens_of(messages, plan.kept()[..plan.stub_slot()].iter());
    match plan.fill() {
        Fill::WithinLow => assert!(projected <= low, "{plan:?}"),
        Fill::NewestExchangeOverLow => assert!(head <= low && projected > low, "{plan:?}"),
        Fill::HeadOverLow => assert!(head > low, "{plan:?}"),
    }
}

/// Plans `messages` at every L under three levels of H, checking each
/// plan; the number of plans made.
fn sweep(messages: &[PlanMessage<'_>]) -> usize {
    let total: usize = messages.iter().map(|m| m.tokens).sum();
    let mut plans = 0;
    for high in [total, total * 3 / 4, total / 2] {
        for low in 1..high {
            let input = bare(messages, (high, low), usize::MAX, None);
            if let Some(plan) = plan_cut(&input) {
                assert_sound(&input, &plan);
                plans += 1;
            }
        }
    }
    plans
}

#[test]
fn a_harness_message_between_a_call_and_its_result_is_no_boundary() {
    let messages = [
        s(10),
        p(10),
        a(&["c1"], 10),
        r("c1", 10),
        a(&["c2"], 10),
        u(10),
        r("c2", 300),
        a(&["c3"], 10),
        r("c3", 10),
        a(&[], 10),
    ];
    assert!(sweep(&messages) > 0);
}

#[test]
fn the_latest_prompt_between_a_call_and_its_result_is_no_boundary() {
    let messages = [
        s(10),
        p(10),
        a(&["c1"], 10),
        r("c1", 10),
        a(&["c2"], 10),
        p(10),
        r("c2", 300),
        a(&["c3"], 10),
        r("c3", 10),
        a(&[], 10),
    ];
    assert!(sweep(&messages) > 0);
}

#[test]
fn interleaved_calls_are_kept_or_archived_together() {
    let messages = [
        s(10),
        p(10),
        a(&["c0"], 10),
        r("c0", 200),
        a(&["c1"], 10),
        a(&["c2"], 10),
        r("c1", 100),
        r("c2", 100),
        a(&[], 10),
    ];
    assert!(sweep(&messages) > 0);
}

#[test]
fn an_unanswered_call_blocks_no_cut() {
    let messages = [
        s(10),
        p(10),
        a(&["lost"], 10),
        a(&["c0"], 10),
        r("c0", 500),
        a(&[], 10),
        u(10),
    ];
    let input = bare(&messages, (560, 40), usize::MAX, None);
    let plan = plan_cut(&input).expect("at H");
    assert_eq!(
        plan.kept()[plan.stub_slot()],
        6,
        "the tail opens at the note"
    );
    assert_sound(&input, &plan);
    assert!(sweep(&messages) > 0);
}

#[test]
fn a_result_without_its_call_is_no_boundary() {
    let messages = [
        s(10),
        p(10),
        a(&["c0"], 10),
        r("c0", 300),
        r("ghost", 50),
        a(&["c1"], 10),
        r("c1", 300),
        a(&[], 10),
    ];
    assert!(sweep(&messages) > 0);
}

#[test]
fn a_system_message_mid_history_stays_before_the_stub() {
    let messages = [
        s(10),
        p(10),
        a(&["c0"], 10),
        r("c0", 500),
        s(20),
        a(&["c1"], 10),
        r("c1", 500),
        a(&["c2"], 10),
        r("c2", 500),
        a(&[], 10),
    ];
    let total = messages.iter().map(|m| m.tokens).sum();
    let input = bare(&messages, (total, 100), usize::MAX, None);
    let plan = plan_cut(&input).expect("at H");
    assert_eq!(plan.kept()[..plan.stub_slot()], [0, 1, 4]);
    let spans: Vec<_> = plan.archived().iter().map(|r| (r.start, r.end)).collect();
    assert_eq!(spans, [(2, 4), (5, 9)]);
    assert_sound(&input, &plan);
}

/// A small linear congruential generator: the same seed, the same cases.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const IDS: [&str; 8] = ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"];

/// A conversation with system messages anywhere, prompts and harness
/// messages, stubs, parallel and interleaved calls, unanswered calls and
/// results without a call.
fn random_conversation(rng: &mut Lcg) -> Vec<PlanMessage<'static>> {
    let mut messages: Vec<_> = (0..rng.below(3)).map(|_| s(rng.below(300) + 1)).collect();
    messages.push(p(rng.below(300) + 1));
    let (mut pending, mut next) = (Vec::new(), 0usize);
    for _ in 0..=rng.below(40) {
        match rng.below(11) {
            0 => messages.push(p(rng.below(500) + 1)),
            1 => messages.push(u(rng.below(100) + 1)),
            2 => messages.push(s(rng.below(50) + 1)),
            3 => messages.push(m(PlanRole::ArchiveStub, 50)),
            4 | 5 => {
                let calls: Vec<&'static str> =
                    (0..rng.below(3)).map(|n| IDS[(next + n) % 8]).collect();
                next += calls.len();
                pending.extend(calls.iter().copied());
                messages.push(m(PlanRole::Assistant { calls }, rng.below(200) + 1));
            }
            6..=8 if !pending.is_empty() => {
                let call = pending.remove(rng.below(pending.len()));
                messages.push(r(call, rng.below(3_000) + 1));
            }
            6..=8 => messages.push(r(IDS[rng.below(8)], 10)),
            9 if !pending.is_empty() => {
                pending.remove(0);
            }
            _ => messages.push(a(&[], rng.below(300) + 1)),
        }
    }
    messages
}

#[test]
fn seeded_random_conversations_keep_every_promise() {
    let mut rng = Lcg(0x2408);
    let mut plans = 0;
    for _ in 0..20_000 {
        let messages = random_conversation(&mut rng);
        let total: usize = messages.iter().map(|m| m.tokens).sum();
        let high = rng.below(total) + 2;
        let low = rng.below(high - 1) + 1;
        let ceiling = match rng.below(4) {
            0 => rng.below(total * 2),
            _ => usize::MAX,
        };
        let after = match rng.below(4) {
            0 => Some(rng.below(total * 2)),
            _ => None,
        };
        let input = CutInput {
            tool_tokens: rng.below(200),
            stub_tokens: rng.below(60) + 1,
            ..bare(&messages, (high, low), ceiling, after)
        };
        if let Some(plan) = plan_cut(&input) {
            assert_sound(&input, &plan);
            plans += 1;
        }
    }
    assert!(
        plans > 2_000,
        "the cases exercise the planner: {plans} plans"
    );
}
