//! Steps for `tui_child_feed_liveness.feature` — the TUI half of the
//! child-progress-freeze fix (2026-07-29): `Command::Sync` bypasses the
//! background writer reserve, and a refused Sync is never recorded in-flight.

use crate::{DebugFeedSender, TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::protocol::client::{
    COMMAND_WRITER_INTERACTIVE_FLOOR, COMMAND_WRITER_QUEUE_CAPACITY, COMMAND_WRITER_USER_RESERVED,
    ClientError, Command, CommandSender,
};
use quecto_tui::shell::app::tui_harness::TuiHarness;

const CHILD: &str = "child-1";

fn background() -> Command {
    Command::GetState { id: None }
}

fn sync() -> Command {
    Command::Sync {
        id: None,
        epoch: 1,
        since_rev: 0,
    }
}

fn sender(world: &mut TuiWorld) -> &CommandSender {
    world
        .tui_feed_liveness_sender
        .as_ref()
        .map(|s| &s.0)
        .expect("queue prepared by a Given step")
}

fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let harness = &mut world.tui_parity.as_mut().expect("harness").0;
    let _guard = handle.enter();
    f(harness)
}

#[given("a production writer queue filled to the background reserve")]
fn queue_filled_to_reserve(world: &mut TuiWorld) {
    let (sender, rx) = CommandSender::production_queue_for_tests();
    for _ in 0..(COMMAND_WRITER_QUEUE_CAPACITY - COMMAND_WRITER_USER_RESERVED) {
        sender.try_send(&background()).expect("within budget");
    }
    world.tui_feed_liveness_sender = Some(DebugFeedSender(sender, rx));
}

#[given("a production writer queue where sync has filled the outer reserve")]
fn queue_sync_filled_to_floor(world: &mut TuiWorld) {
    let (sender, rx) = CommandSender::production_queue_for_tests();
    for _ in 0..(COMMAND_WRITER_QUEUE_CAPACITY - COMMAND_WRITER_INTERACTIVE_FLOOR) {
        sender
            .try_send(&sync())
            .expect("fills to the interactive floor");
    }
    world.tui_feed_liveness_sender = Some(DebugFeedSender(sender, rx));
}

#[then("an interactive command should still be accepted")]
fn interactive_accepted(world: &mut TuiWorld) {
    sender(world)
        .try_send(&Command::Abort { id: None })
        .expect("the interactive floor belongs to user commands");
}

#[then("a further background command should be refused with backpressure")]
fn background_refused(world: &mut TuiWorld) {
    assert!(matches!(
        sender(world).try_send(&background()),
        Err(ClientError::Backpressure)
    ));
}

#[then("a sync command should still be accepted")]
fn sync_accepted(world: &mut TuiWorld) {
    sender(world)
        .try_send(&sync())
        .expect("Sync must bypass the background reserve — a refused Sync freezes the child feed");
}

#[then("a further sync command should be refused with backpressure")]
fn sync_refused_when_full(world: &mut TuiWorld) {
    assert!(
        matches!(
            sender(world).try_send(&sync()),
            Err(ClientError::Backpressure)
        ),
        "sync must stop at the interactive floor"
    );
}

#[given("a tracked child feed whose command channel is full")]
fn tracked_full_feed(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut harness = rt.block_on(TuiHarness::new());
    let _guard = rt.handle().clone();
    let rx = {
        let _enter = rt.enter();
        harness.insert_full_channel_feed(CHILD)
    };
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(harness));
    world.tui_feed_liveness_child_rx = Some(rx);
}

#[given("a ledger advance hint arrives for that child")]
#[when("a ledger advance hint arrives for that child")]
fn ledger_hint_arrives(world: &mut TuiWorld) {
    drive(world, |h| h.note_child_ledger_advanced(CHILD, 1, 9));
}

#[then("no sync should be recorded as in-flight")]
fn no_phantom_pending(world: &mut TuiWorld) {
    let pending = drive(world, |h| h.child_feed_pending_rev(CHILD));
    assert_eq!(
        pending, None,
        "a refused Sync recorded in-flight is a phantom sync that never resolves"
    );
}

#[when("the channel frees a slot and a newer ledger hint arrives")]
fn slot_frees_and_newer_hint(world: &mut TuiWorld) {
    world
        .tui_feed_liveness_child_rx
        .as_mut()
        .expect("child feed channel")
        .try_recv()
        .expect("drain prefill");
    drive(world, |h| h.note_child_ledger_advanced(CHILD, 1, 10));
}

#[then("a sync command should be enqueued for the child")]
fn sync_enqueued(world: &mut TuiWorld) {
    let cmd = world
        .tui_feed_liveness_child_rx
        .as_mut()
        .expect("child feed channel")
        .try_recv()
        .expect("retry sync after refusal");
    assert!(matches!(
        cmd,
        Command::Sync {
            epoch: 1,
            since_rev: 0,
            ..
        }
    ));
}

#[then("the newer revision should be recorded as in-flight")]
fn newer_rev_pending(world: &mut TuiWorld) {
    let pending = drive(world, |h| h.child_feed_pending_rev(CHILD));
    assert_eq!(pending, Some(10));
}
