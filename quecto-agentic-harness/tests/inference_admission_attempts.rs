//! #1679 P2 RED: actual leaf sends, not a decorated/fake LlmProvider.
//! Each macro expansion is an independently executable test (including both
//! Responses auth modes and Codex's assembled-stream default delegation).
//! Wall timeouts are cleanup/failure bounds, never absence-of-send oracles.
#[path = "common/admission_attempt_fixture.rs"]
mod fixture;

use std::sync::Arc;

use fixture::*;
use quecto::domain::provider::{CancelFlag, StreamEvent};

async fn one_attempt(leaf: Leaf, surface: Surface) {
    let gate = Gate::new(false);
    let mut server = Server::start(leaf, surface, 1, Pause::None, gate.clone()).await;
    let provider = leaf.provider(&server.url, gate.clone());
    oracle::text(
        &bounded(invoke(provider.as_ref(), surface)).await.unwrap(),
        "0,",
    );
    let _connection = server.connection().await;
    gate.assert_exact(1);
    gate.wait_event(Event::Finished(0)).await;
    oracle::active(gate.active(), 0);
    server.shutdown().await;
}

macro_rules! surfaces {
    ($module:ident, $leaf:expr) => {
        mod $module {
            use super::*;
            #[tokio::test]
            async fn chat_charges_exactly_one_before_send() {
                one_attempt($leaf, Surface::Chat).await;
            }
            #[tokio::test]
            async fn assembled_stream_charges_exactly_one_before_send() {
                one_attempt($leaf, Surface::Stream).await;
            }
            #[tokio::test]
            async fn incremental_charges_exactly_one_before_send() {
                one_attempt($leaf, Surface::Incremental).await;
            }
            #[tokio::test]
            async fn returned_receiver_keeps_capacity_occupied() {
                receiver_lifetime($leaf).await;
            }
            #[tokio::test]
            async fn slow_consumer_retains_permit_and_order_beyond_64_deltas() {
                backpressure($leaf).await;
            }
            #[tokio::test]
            async fn cancel_queued_chat_never_sends() {
                cancel_queued($leaf, Surface::Chat).await;
            }
            #[tokio::test]
            async fn cancel_queued_assembled_stream_never_sends() {
                cancel_queued($leaf, Surface::Stream).await;
            }
            #[tokio::test]
            async fn cancel_queued_incremental_never_sends() {
                cancel_queued($leaf, Surface::Incremental).await;
            }
            #[tokio::test]
            async fn cancel_at_grant_acknowledges_without_send() {
                granted_no_send($leaf).await;
            }
            #[tokio::test]
            async fn dropped_receiver_cancels_blocked_headers() {
                drop_receiver($leaf, Pause::Headers, 0).await;
            }
            #[tokio::test]
            async fn dropped_receiver_cancels_blocked_body() {
                drop_receiver($leaf, Pause::Body, 0).await;
            }
            #[tokio::test]
            async fn dropped_receiver_cancels_full_channel_send() {
                drop_receiver($leaf, Pause::Body, 130).await;
            }
            #[tokio::test]
            async fn abort_chat_destroys_transport_before_replacement() {
                abort_call($leaf, Surface::Chat, Pause::Headers).await;
            }
            #[tokio::test]
            async fn abort_assembled_stream_with_blocked_body() {
                abort_call($leaf, Surface::Stream, Pause::Body).await;
            }
        }
    };
}
surfaces!(openai, Leaf::OpenAi);
surfaces!(responses_api_key, Leaf::Responses);
surfaces!(codex_oauth, Leaf::CodexOAuth);
surfaces!(anthropic, Leaf::Anthropic);

async fn receiver_lifetime(leaf: Leaf) {
    let gate = Gate::new(false);
    let mut server = Server::start(leaf, Surface::Incremental, 1, Pause::Body, gate.clone()).await;
    let provider = leaf.provider(&server.url, gate.clone());
    let mut first = bounded(provider.chat_stream_incremental(request())).await;
    let first_connection = server.connection().await;
    oracle::delta(bounded(first.recv()).await, Some("0,"));
    let sibling_provider = Arc::clone(&provider);
    let mut acquiring = Task(tokio::spawn(async move {
        sibling_provider.chat_stream_incremental(request()).await
    }));
    gate.wait_event(Event::Queued(1)).await;
    // Queue registration is an executor barrier. No sleep-based negative oracle.
    oracle::active(gate.active(), 1);
    oracle::trace(
        &gate.events(),
        &[
            Event::Queued(0),
            Event::Granted(0),
            Event::RawStart(0),
            Event::Queued(1),
        ],
    );
    first_connection.finish().await;
    oracle::done(bounded(first.recv()).await, None);
    oracle::closed(bounded(first.recv()).await);
    let second_connection = server.connection().await;
    let mut sibling = bounded(&mut acquiring.0).await.unwrap();
    oracle::delta(bounded(sibling.recv()).await, None);
    second_connection.finish().await;
    oracle::done(bounded(sibling.recv()).await, None);
    oracle::closed(bounded(sibling.recv()).await);
    gate.wait_event(Event::Finished(1)).await;
    gate.assert_exact(2);
    oracle::replacement_after_finish(&gate.events());
    server.shutdown().await;
}

async fn full(rx: &tokio::sync::mpsc::Receiver<StreamEvent>) {
    oracle::capacity(rx.max_capacity(), 64);
    bounded(async {
        while rx.len() != 64 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    oracle::capacity(rx.capacity(), 0);
}

async fn backpressure(leaf: Leaf) {
    let gate = Gate::new(false);
    let mut server =
        Server::start(leaf, Surface::Incremental, 130, Pause::Body, gate.clone()).await;
    let provider = leaf.provider(&server.url, gate.clone());
    let mut rx = bounded(provider.chat_stream_incremental(request())).await;
    let connection = server.connection().await;
    full(&rx).await;
    oracle::active(gate.active(), 1);
    oracle::absent(&gate.events(), Event::Finished(0));
    for i in 0..130 {
        oracle::delta(bounded(rx.recv()).await, Some(&format!("{i},")));
    }
    oracle::active(gate.active(), 1);
    connection.finish().await;
    oracle::done(
        bounded(rx.recv()).await,
        Some(&(0..130).map(|i| format!("{i},")).collect::<String>()),
    );
    oracle::closed(bounded(rx.recv()).await);
    gate.wait_event(Event::Finished(0)).await;
    gate.assert_exact(1);
    oracle::active(gate.active(), 0);
    server.shutdown().await;
}

async fn cancel_queued(leaf: Leaf, surface: Surface) {
    let gate = Gate::new(true);
    let server = Server::start(leaf, surface, 1, Pause::None, gate.clone()).await;
    let provider = leaf.provider(&server.url, gate.clone());
    let flag = CancelFlag::new();
    let task_flag = flag.clone();
    let mut call = Task(tokio::spawn(async move {
        let mut req = request();
        req.cancel_flag = Some(task_flag);
        match surface {
            Surface::Chat => {
                let _ = provider.chat(req).await;
            }
            Surface::Stream => {
                let _ = provider.chat_stream(req).await;
            }
            Surface::Incremental => {
                let mut rx = provider.chat_stream_incremental(req).await;
                // Keep receiver alive until cancellation, irrespective of whether
                // the implementation returns it before or after acquiring.
                while rx.recv().await.is_some() {}
            }
        }
    }));
    gate.wait_event(Event::Queued(0)).await;
    flag.cancel();
    // Cancellation must drop the queued acquire, not leave an orphan waiter
    // which will dispatch as soon as unrelated capacity becomes available.
    gate.wait_event(Event::QueueCancelled(0)).await;
    bounded(&mut call.0).await.unwrap();
    gate.open();
    oracle::trace(
        &gate.events(),
        &[Event::Queued(0), Event::QueueCancelled(0)],
    );
    gate.assert_exact(0);
    oracle::active(gate.active(), 0);
    server.shutdown().await;
}

async fn granted_no_send(leaf: Leaf) {
    let gate = Gate::new(false);
    let server = Server::start(leaf, Surface::Chat, 1, Pause::None, gate.clone()).await;
    let provider = leaf.provider(&server.url, gate.clone());
    let flag = CancelFlag::new();
    gate.cancel_on_grant(flag.clone());
    let mut req = request();
    req.cancel_flag = Some(flag);
    oracle::cancelled(bounded(provider.chat(req)).await.is_err());
    gate.wait_event(Event::Finished(0)).await;
    oracle::trace(
        &gate.events(),
        &[Event::Queued(0), Event::Granted(0), Event::Finished(0)],
    );
    oracle::active(gate.active(), 0);
    server.shutdown().await;
}

async fn drop_receiver(leaf: Leaf, pause: Pause, count: usize) {
    let gate = Gate::new(false);
    let mut server = Server::start(leaf, Surface::Incremental, count, pause, gate.clone()).await;
    let provider = leaf.provider(&server.url, gate.clone());
    let rx = bounded(provider.chat_stream_incremental(request())).await;
    let first = server.connection().await;
    if count > 64 {
        full(&rx).await;
    }
    let sibling_provider = Arc::clone(&provider);
    let mut acquiring = Task(tokio::spawn(async move {
        sibling_provider.chat_stream_incremental(request()).await
    }));
    gate.wait_event(Event::Queued(1)).await;
    oracle::active(gate.active(), 1);
    oracle::absent(&gate.events(), Event::Granted(1));
    drop(rx);
    // Receiver destruction itself cannot acknowledge task destruction. On this
    // current-thread executor no owned task has run since drop(rx).
    oracle::active(gate.active(), 1);
    first.eof().await;
    gate.wait_event(Event::Finished(0)).await;
    let second = server.connection().await;
    let mut sibling = bounded(&mut acquiring.0).await.unwrap();
    second.finish().await;
    while let Some(event) = bounded(sibling.recv()).await {
        oracle::not_error(&event);
    }
    gate.wait_event(Event::Finished(1)).await;
    gate.assert_exact(2);
    oracle::active(gate.active(), 0);
    oracle::present(&gate.events(), Event::PeerEof(0));
    server.shutdown().await;
}

async fn abort_call(leaf: Leaf, surface: Surface, pause: Pause) {
    let gate = Gate::new(false);
    let mut server = Server::start(leaf, surface, 1, pause, gate.clone()).await;
    let provider = leaf.provider(&server.url, gate.clone());
    let called = Arc::clone(&provider);
    let mut task = Task(tokio::spawn(async move {
        invoke(called.as_ref(), surface).await
    }));
    let first = server.connection().await;
    let sibling_provider = Arc::clone(&provider);
    let mut sibling = Task(tokio::spawn(async move {
        invoke(sibling_provider.as_ref(), surface).await
    }));
    gate.wait_event(Event::Queued(1)).await;
    task.0.abort();
    oracle::active(gate.active(), 1);
    oracle::absent(&gate.events(), Event::Granted(1));
    oracle::cancelled(bounded(&mut task.0).await.unwrap_err().is_cancelled());
    first.eof().await;
    gate.wait_event(Event::Finished(0)).await;
    let second = server.connection().await;
    second.finish().await;
    oracle::text(&bounded(&mut sibling.0).await.unwrap().unwrap(), "0,");
    gate.wait_event(Event::Finished(1)).await;
    gate.assert_exact(2);
    oracle::active(gate.active(), 0);
    server.shutdown().await;
}

/// Minimal deadline capability contracts; the larger leaf/surface matrix above
/// remains independent. Authority signals are explicit, not wall-clock sleeps.
#[tokio::test]
async fn queue_deadline_rejection_never_grants_or_sends() {
    let gate = Gate::new(true);
    let server = Server::start(Leaf::OpenAi, Surface::Chat, 1, Pause::None, gate.clone()).await;
    let provider = Leaf::OpenAi.provider(&server.url, gate.clone());
    let mut task = Task(tokio::spawn(async move { provider.chat(request()).await }));
    gate.wait_event(Event::Queued(0)).await;
    gate.expire_queue();
    oracle::cancelled(bounded(&mut task.0).await.unwrap().is_err());
    gate.open();
    oracle::trace(&gate.events(), &[Event::Queued(0), Event::QueueExpired(0)]);
    gate.assert_exact(0);
    oracle::active(gate.active(), 0);
    server.shutdown().await;
}

#[tokio::test]
async fn active_deadline_requires_owned_transport_shutdown_ack() {
    let gate = Gate::new(false);
    let mut server = Server::start(
        Leaf::OpenAi,
        Surface::Incremental,
        0,
        Pause::Body,
        gate.clone(),
    )
    .await;
    let provider = Leaf::OpenAi.provider(&server.url, gate.clone());
    let mut receiver = bounded(provider.chat_stream_incremental(request())).await;
    let connection = server.connection().await;
    gate.wait_event(Event::Granted(0)).await;
    gate.expire_active(0);
    // The authority has signalled, but the current-thread executor has not run
    // the owner. Deadline intent is no stronger release evidence than abort intent.
    oracle::active(gate.active(), 1);
    oracle::absent(&gate.events(), Event::Finished(0));
    connection.eof().await;
    oracle::deadline_error(bounded(receiver.recv()).await);
    oracle::closed(bounded(receiver.recv()).await);
    gate.wait_event(Event::Finished(0)).await;
    gate.assert_exact(1);
    oracle::active(gate.active(), 0);
    oracle::present(&gate.events(), Event::PeerEof(0));
    oracle::present(&gate.events(), Event::ActiveExpired(0));
    server.shutdown().await;
}

/// Synthetic observation perturbations exercise the SAME pure assertions used
/// above. These are oracle falsifiability checks, not fake provider enforcement,
/// transport tests, production GREEN, or post-GREEN algorithm mutation evidence.
mod oracle_counterexamples {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn rejects(label: &str, assertion: impl FnOnce()) {
        assert!(
            catch_unwind(AssertUnwindSafe(assertion)).is_err(),
            "counterexample escaped the production-test oracle: {label}"
        );
    }
    fn done(text: &str) -> StreamEvent {
        StreamEvent::Done(quecto::domain::message::LlmResponse {
            content: Some(text.into()),
            tool_calls: vec![],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        })
    }
    fn delta(text: &str) -> StreamEvent {
        StreamEvent::TextDelta(text.into())
    }

    #[test]
    fn counts_order_identity_and_uncertain_drop_are_falsifiable() {
        use Event::*;
        let valid = [Queued(0), Granted(0), RawStart(0), Finished(0)];
        oracle::exact(&valid, 1);
        oracle::granted_before_send(&valid);
        oracle::exact(&[], 0);
        rejects("missing grant", || oracle::exact(&[RawStart(0)], 1));
        rejects("double grant", || {
            oracle::exact(&[Granted(0), Granted(0), RawStart(0)], 1)
        });
        rejects("missing raw start", || oracle::exact(&[Granted(0)], 1));
        rejects("double raw start", || {
            oracle::exact(&[Granted(0), RawStart(0), RawStart(0)], 1)
        });
        rejects("grant after send", || {
            oracle::exact(&[RawStart(0), Granted(0)], 1)
        });
        rejects("wrong grant identity", || {
            oracle::exact(&[Granted(1), RawStart(0)], 1)
        });
        rejects("wrong send identity", || {
            oracle::exact(&[Granted(0), RawStart(1)], 1)
        });
        rejects("uncertain drop", || {
            oracle::exact(&[Granted(0), RawStart(0), Abandoned(0)], 1)
        });
        rejects("wait barrier observes ungranted send", || {
            oracle::granted_before_send(&[RawStart(0)])
        });
        rejects("wait barrier observes late grant", || {
            oracle::granted_before_send(&[RawStart(0), Granted(0)])
        });
        rejects("unexpected granted queued cancellation", || {
            oracle::exact(&[Granted(0)], 0)
        });
        rejects("unexpected ungranted queued send", || {
            oracle::exact(&[RawStart(0)], 0)
        });
    }

    #[test]
    fn lifetime_backpressure_and_replacement_traces_are_falsifiable() {
        use Event::*;
        let held = [Queued(0), Granted(0), RawStart(0), Queued(1)];
        oracle::active(1, 1);
        oracle::active(0, 0);
        oracle::trace(&held, &held);
        oracle::absent(&held, Finished(0));
        oracle::absent(&held, Granted(1));
        rejects("release on receiver return", || oracle::active(0, 1));
        rejects("release on receiver destructor", || oracle::active(0, 1));
        rejects("release on abort request", || oracle::active(0, 1));
        rejects("release on full channel", || oracle::active(0, 1));
        rejects("release after delta drain before EOF", || {
            oracle::active(0, 1)
        });
        rejects("leak after transport finish", || oracle::active(1, 0));
        let mut premature = held.to_vec();
        premature.push(Finished(0));
        rejects("finish before body terminal", || {
            oracle::absent(&premature, Finished(0))
        });
        rejects("receiver return releases and changes lifecycle", || {
            oracle::trace(&premature, &held)
        });
        premature.push(Granted(1));
        rejects("sibling admitted too early", || {
            oracle::absent(&premature, Granted(1))
        });
        oracle::replacement_after_finish(&[Finished(0), Granted(1)]);
        rejects("replacement before finish", || {
            oracle::replacement_after_finish(&[Granted(1), Finished(0)])
        });
        rejects("missing original finish", || {
            oracle::replacement_after_finish(&[Granted(1)])
        });
        rejects("missing replacement grant", || {
            oracle::replacement_after_finish(&[Finished(0)])
        });
        oracle::capacity(64, 64);
        oracle::capacity(0, 0);
        rejects("changed channel bound", || oracle::capacity(65, 64));
        rejects("not actually backpressured", || oracle::capacity(1, 0));
    }

    #[test]
    fn cancellation_ack_and_peer_eof_are_falsifiable() {
        use Event::*;
        let cancelled = [Queued(0), QueueCancelled(0)];
        let no_send = [Queued(0), Granted(0), Finished(0)];
        oracle::trace(&cancelled, &cancelled);
        oracle::trace(&no_send, &no_send);
        rejects("orphan queued waiter", || {
            oracle::trace(&[Queued(0)], &cancelled)
        });
        rejects("cancelled queue later sends", || {
            oracle::trace(
                &[Queued(0), QueueCancelled(0), Granted(0), RawStart(0)],
                &cancelled,
            )
        });
        rejects("grant refunded as queue cancellation", || {
            oracle::trace(&cancelled, &no_send)
        });
        rejects("grant not acknowledged", || {
            oracle::trace(&[Queued(0), Granted(0)], &no_send)
        });
        rejects("send despite cancelled grant", || {
            oracle::trace(&[Queued(0), Granted(0), RawStart(0), Finished(0)], &no_send)
        });
        rejects("double finish", || {
            oracle::trace(&[Queued(0), Granted(0), Finished(0), Finished(0)], &no_send)
        });
        oracle::cancelled(true);
        rejects("successful call despite cancel", || {
            oracle::cancelled(false)
        });
        rejects("aborted JoinHandle not cancelled", || {
            oracle::cancelled(false)
        });
        for event in [
            Queued(0),
            Queued(1),
            QueueCancelled(0),
            Finished(0),
            Finished(1),
            PeerEof(0),
        ] {
            oracle::present(std::slice::from_ref(&event), event.clone());
            rejects("missing required lifecycle observation", || {
                oracle::present(&[], event)
            });
        }
        oracle::eof(0);
        rejects("socket remains readable rather than EOF", || oracle::eof(1));
    }

    #[test]
    fn stream_content_terminal_and_closure_are_falsifiable() {
        oracle::text("0,", "0,");
        rejects("wrong successful content", || oracle::text("", "0,"));
        for i in 0..130 {
            let expected = format!("{i},");
            oracle::delta(Some(delta(&expected)), Some(&expected));
            rejects("lost or reordered numbered delta", || {
                oracle::delta(Some(delta(&format!("{},", i + 1))), Some(&expected))
            });
        }
        oracle::delta(Some(delta("0,")), None);
        rejects("non-delta in delta position", || {
            oracle::delta(Some(done("0,")), None)
        });
        rejects("missing delta", || oracle::delta(None, Some("0,")));
        let content = (0..130).map(|i| format!("{i},")).collect::<String>();
        oracle::done(Some(done(&content)), Some(&content));
        oracle::done(Some(done("0,")), None);
        rejects("truncated assembled content", || {
            oracle::done(Some(done("0,")), Some(&content))
        });
        rejects("nonterminal instead of Done", || {
            oracle::done(Some(delta("0,")), None)
        });
        rejects("missing Done", || oracle::done(None, None));
        oracle::closed(None);
        rejects("extra delta after Done", || {
            oracle::closed(Some(delta("extra")))
        });
        rejects("extra Done", || oracle::closed(Some(done("0,"))));
        oracle::not_error(&delta("0,"));
        rejects("sibling error", || {
            oracle::not_error(&StreamEvent::Error("failure".into()))
        });
        oracle::done_count(false);
        rejects("duplicate terminal event in invoke", || {
            oracle::done_count(true)
        });
        assert_eq!(oracle::terminal(Some("0,".into())), "0,");
        rejects("invoke lacks terminal event", || {
            oracle::terminal(None);
        });
        rejects("incremental and assembled mismatch", || {
            oracle::text("0,1,", "0,")
        });
    }

    #[test]
    fn deadline_rejection_and_shutdown_observations_are_falsifiable() {
        use Event::*;
        let rejected = [Queued(0), QueueExpired(0)];
        oracle::trace(&rejected, &rejected);
        rejects("queue rejection silently cancelled", || {
            oracle::trace(&[Queued(0), QueueCancelled(0)], &rejected)
        });
        rejects("queue rejection nonetheless granted", || {
            oracle::trace(&[Queued(0), QueueExpired(0), Granted(0)], &rejected)
        });
        rejects("queue rejection nonetheless sent", || {
            oracle::trace(&[Queued(0), QueueExpired(0), RawStart(0)], &rejected)
        });
        oracle::present(&[ActiveExpired(0)], ActiveExpired(0));
        rejects("missing authority expiry", || {
            oracle::present(&[], ActiveExpired(0))
        });
        rejects("deadline signal releases before owner", || {
            oracle::active(0, 1)
        });
        rejects("deadline signal finishes before owner", || {
            oracle::absent(&[ActiveExpired(0), Finished(0)], Finished(0))
        });
        oracle::deadline_error(Some(StreamEvent::Error(
            "admission attempt deadline expired".into(),
        )));
        rejects("deadline silently ends stream", || {
            oracle::deadline_error(None)
        });
        rejects("deadline falsely succeeds", || {
            oracle::deadline_error(Some(done("")))
        });
    }

    #[test]
    fn fixture_protocol_safeguards_are_falsifiable() {
        oracle::request_method("POST /responses HTTP/1.1");
        rejects("wrong HTTP method", || {
            oracle::request_method("GET /responses HTTP/1.1")
        });
        for cap in [32_768, 1_048_576] {
            oracle::request_size(cap - 1, cap);
            rejects("request bound exceeded", || oracle::request_size(cap, cap));
        }
    }

    #[tokio::test]
    async fn bounded_liveness_oracle_rejects_nonterminating_observation() {
        use futures::FutureExt;
        // The same deadline wrapper bounds recv, queue transition, server EOF,
        // task join and completion. One shared nontermination perturbation tests
        // that failure path; no wall-clock absence-of-send claim is made.
        bounded(async {}).await;
        assert!(
            AssertUnwindSafe(bounded(std::future::pending::<()>()))
                .catch_unwind()
                .await
                .is_err()
        );
    }
}
