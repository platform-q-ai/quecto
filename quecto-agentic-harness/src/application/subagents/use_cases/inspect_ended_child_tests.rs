use super::*;
use crate::application::subagents::ports::{EndedTranscript, PortFuture};
use crate::domain::agents::child_end::ChildOrigin::{Launched, Reported, Unverified};
use crate::domain::error::DomainError;
use crate::domain::sessions::entities::session::Session;
use std::sync::Mutex;

/// Records answering one scripted transcript load and a fixed crash record,
/// noting which sessions were read.
struct Store {
    answer: Mutex<Option<Result<Option<Session>, DomainError>>>,
    read: Mutex<Vec<String>>,
}

impl Store {
    fn answering(answer: Result<Option<Session>, DomainError>) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(Some(answer)),
            read: Mutex::new(Vec::new()),
        })
    }
}

struct Records {
    store: Arc<Store>,
    crash: Option<CrashRecord>,
    crash_reads: Mutex<Vec<(String, Option<u32>)>>,
}

impl EndedChildRecords for Records {
    fn crash<'a>(
        &'a self,
        child: &'a SessionIdentity,
        writer: Option<u32>,
    ) -> PortFuture<'a, Option<CrashRecord>> {
        self.crash_reads
            .lock()
            .unwrap()
            .push((child.runtime_key().to_owned(), writer));
        let record = self.crash.clone();
        Box::pin(async move { record })
    }

    fn transcript<'a>(
        &'a self,
        child: &'a SessionIdentity,
    ) -> PortFuture<'a, Result<Option<EndedTranscript>, DomainError>> {
        self.store
            .read
            .lock()
            .unwrap()
            .push(child.runtime_key().to_owned());
        let answer = self.store.answer.lock().unwrap().take().expect("one load");
        Box::pin(async move {
            answer.map(|session| {
                session.map(|s| EndedTranscript {
                    messages: Arc::new(s.messages),
                    older_omitted: false,
                })
            })
        })
    }
}

fn inspect(store: Arc<Store>, crash: Option<CrashRecord>) -> (InspectEndedChild, Arc<Records>) {
    let records = Arc::new(Records {
        store,
        crash,
        crash_reads: Mutex::new(Vec::new()),
    });
    (
        InspectEndedChild::new(records.clone(), "/base/sessions"),
        records,
    )
}

fn child() -> AgentUuid {
    AgentUuid::new("65268567-be4a-471f-a805-1238dcf08b68")
}

fn transcript(texts: &[&str]) -> Session {
    let identity = InspectEndedChild::child_session(&child()).unwrap();
    let mut session = Session::new(identity);
    session.messages = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let mut message = Message::user(*text);
            message.ordinal = Some(i as u64 + 1);
            message
        })
        .collect();
    session
}

#[test]
fn a_launched_child_runs_as_its_cli_session() {
    let identity = InspectEndedChild::child_session(&child()).unwrap();
    assert_eq!(
        identity.runtime_key(),
        "cli:65268567-be4a-471f-a805-1238dcf08b68"
    );
    assert!(InspectEndedChild::child_session(&AgentUuid::new("not a uuid!")).is_none());
}

#[tokio::test]
async fn the_persisted_transcript_is_paged_from_the_childs_session() {
    let store = Store::answering(Ok(Some(transcript(&["one", "two", "three"]))));
    let (inspect, _) = inspect(store.clone(), None);
    let page = inspect
        .transcript(&child(), Launched, 2, None)
        .await
        .unwrap();
    let texts: Vec<&str> = page.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(texts, ["two", "three"]);
    assert!(page.has_more_before);
    assert_eq!(page.before, Some(2), "the cursor is the durable ordinal");
    assert_eq!(
        *store.read.lock().unwrap(),
        ["cli:65268567-be4a-471f-a805-1238dcf08b68"]
    );
}

#[tokio::test]
async fn an_older_window_continues_before_the_cursor_to_the_start() {
    let store = Store::answering(Ok(Some(transcript(&["one", "two", "three"]))));
    let (inspect, _) = inspect(store, None);
    let page = inspect
        .transcript(&child(), Launched, 5, Some(2))
        .await
        .unwrap();
    let texts: Vec<&str> = page.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(texts, ["one"]);
    assert!(!page.has_more_before);
    assert_eq!(page.before, None);
}

#[tokio::test]
async fn a_transcript_the_store_does_not_hold_says_where_it_looked() {
    let (inspect, _) = inspect(Store::answering(Ok(None)), None);
    let error = inspect
        .transcript(&child(), Launched, 5, None)
        .await
        .unwrap_err();
    let EndedTranscriptError::NotHere(reason) = &error else {
        panic!("{error:?}")
    };
    assert!(
        reason.contains("cli:65268567-be4a-471f-a805-1238dcf08b68")
            && reason.contains("/base/sessions")
            && reason.contains("container"),
        "{reason}"
    );
}

#[tokio::test]
async fn a_store_failure_is_unreadable_not_absent() {
    let (inspect, _) = inspect(
        Store::answering(Err(DomainError::Session("corrupt".into()))),
        None,
    );
    let error = inspect
        .transcript(&child(), Launched, 5, None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, EndedTranscriptError::Unreadable(reason) if reason.contains("corrupt")),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_unknown_cursor_is_refused() {
    let (inspect, _) = inspect(Store::answering(Ok(Some(transcript(&["one"])))), None);
    let error = inspect
        .transcript(&child(), Launched, 5, Some(9))
        .await
        .unwrap_err();
    assert_eq!(error, EndedTranscriptError::UnknownCursor(9));
    assert_eq!(
        error.to_string(),
        "no message with ordinal 9 in its persisted transcript"
    );
}

#[tokio::test]
async fn a_child_that_is_not_a_launched_identity_has_nothing_to_read() {
    let (inspect, crashes) = inspect(Store::answering(Ok(None)), None);
    let odd = AgentUuid::new("has space");
    let error = inspect
        .transcript(&odd, Launched, 1, None)
        .await
        .unwrap_err();
    assert!(matches!(error, EndedTranscriptError::NotHere(_)));
    assert_eq!(inspect.crash(&odd, Launched, Some(7)).await, None);
    assert!(
        crashes.crash_reads.lock().unwrap().is_empty(),
        "no record is read"
    );
}

#[tokio::test]
async fn the_crash_record_is_read_for_the_childs_session() {
    let record = CrashRecord::new(
        crate::domain::crash_record::PanicReport::new("boom", None),
        1,
        2,
    )
    .running(vec!["edit".into()]);
    let (inspect, crashes) = inspect(Store::answering(Ok(None)), Some(record.clone()));
    assert_eq!(
        inspect.crash(&child(), Launched, Some(7)).await,
        Some(record)
    );
    assert_eq!(
        *crashes.crash_reads.lock().unwrap(),
        [(
            "cli:65268567-be4a-471f-a805-1238dcf08b68".to_string(),
            Some(7)
        )],
        "a launched child's own process is asked for"
    );
}

#[tokio::test]
async fn a_transcript_without_ordinals_is_numbered_by_position() {
    let mut session = transcript(&["one", "two", "three"]);
    for message in &mut session.messages {
        message.ordinal = None;
    }
    let (inspect, _) = inspect(Store::answering(Ok(Some(session))), None);
    let page = inspect
        .transcript(&child(), Launched, 1, None)
        .await
        .unwrap();
    assert_eq!(page.messages[0].content, "three");
    assert_eq!(page.before, Some(3), "a cursor to page on by");
    assert!(page.has_more_before);
}

/// A transcript only partly numbered (older messages saved before ordinals)
/// pages by position throughout, so every message has a cursor.
#[tokio::test]
async fn a_partly_numbered_transcript_is_numbered_by_position() {
    let mut session = transcript(&["one", "two", "three"]);
    session.messages[0].ordinal = None;
    session.messages[1].ordinal = Some(7);
    session.messages[2].ordinal = Some(10);
    let (newest, _) = inspect(Store::answering(Ok(Some(session.clone()))), None);
    let page = newest
        .transcript(&child(), Launched, 2, None)
        .await
        .unwrap();
    assert_eq!(page.before, Some(2), "positions, not the stray ordinal");
    let (again, _) = inspect(Store::answering(Ok(Some(session))), None);
    let older = again
        .transcript(&child(), Launched, 2, page.before)
        .await
        .unwrap();
    assert_eq!(older.messages[0].content, "one");
    assert!(!older.has_more_before);
}

/// #2192 review M2: a child cannot make its parent read another session's
/// transcript by reporting a "descendant" named after it (`secret-plan`
/// is a valid agent id, and `named_cli` would turn it into that session).
/// Only a child this harness launched has its transcript read.
#[tokio::test]
async fn a_reported_or_unverified_child_has_no_transcript_read_on_its_word() {
    for (origin, name) in [
        (Reported, "secret-plan"),
        (Reported, "65268567-be4a-471f-a805-1238dcf08b68"),
        (Unverified, "secret-plan"),
        (Unverified, "65268567-be4a-471f-a805-1238dcf08b68"),
    ] {
        let store = Store::answering(Ok(Some(transcript(&["the secret plan"]))));
        let (inspect, _) = inspect(store.clone(), None);
        let error = inspect
            .transcript(&AgentUuid::new(name), origin, 5, None)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, EndedTranscriptError::NotHere(why)
                if why.contains("did not launch")),
            "{origin:?} {name}: {error:?}"
        );
        assert!(
            store.read.lock().unwrap().is_empty(),
            "{origin:?} {name}: no session is loaded"
        );
    }
}

/// #2192 review M1/M2: a reported child's crash record is read only under
/// a minted-form uuid, and never as a known process's (nothing of it can
/// then be believed); a launched child's is asked for by its own pid.
#[tokio::test]
async fn a_reported_childs_crash_record_is_read_only_by_uuid_and_never_as_its_process() {
    let record = CrashRecord::new(
        crate::domain::crash_record::PanicReport::new("boom", None),
        999_999,
        2,
    );
    let (inspect, crashes) = inspect(Store::answering(Ok(None)), Some(record.clone()));
    for origin in [Reported, Unverified] {
        assert_eq!(
            inspect
                .crash(&AgentUuid::new("secret-plan"), origin, Some(7))
                .await,
            None
        );
    }
    assert!(crashes.crash_reads.lock().unwrap().is_empty());
    assert_eq!(
        inspect.crash(&child(), Reported, Some(7)).await,
        Some(record)
    );
    assert_eq!(
        *crashes.crash_reads.lock().unwrap(),
        [("cli:65268567-be4a-471f-a805-1238dcf08b68".to_string(), None)],
        "no pid is vouched for"
    );
}

/// #2192 review: a page never copies more than one page's bound of
/// messages, however large a `count` is asked for.
#[tokio::test]
async fn a_huge_count_copies_at_most_one_page() {
    let texts: Vec<String> = (0..(InspectEndedChild::MAX_PAGE_MESSAGES + 50))
        .map(|n| format!("m{n}"))
        .collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let (inspect, _) = inspect(Store::answering(Ok(Some(transcript(&refs)))), None);
    let page = inspect
        .transcript(&child(), Launched, usize::MAX, None)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), InspectEndedChild::MAX_PAGE_MESSAGES);
    assert!(page.has_more_before);
    assert_eq!(
        page.messages.last().unwrap().content,
        format!("m{}", InspectEndedChild::MAX_PAGE_MESSAGES + 49)
    );
}
