use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, person, task};
use crate::domain::project_board::value_objects::vocabulary::{DiscussionState, TaskStatus};

fn message(who: &str, time: &str, text: &str) -> Message {
    Message {
        author: person(who),
        at: at(time),
        text: text.into(),
    }
}

fn when(time: &str) -> Moment {
    Moment {
        now: at(time),
        latest: None,
    }
}

fn bounded(now: &str, latest: &str) -> Moment {
    Moment {
        now: at(now),
        latest: Some(at(latest)),
    }
}

fn id() -> Slug {
    Slug::parse("store-locking").unwrap()
}

fn opening(messages: Vec<Message>) -> Opening<'static> {
    Opening {
        id: id(),
        title: "How the store locks".into(),
        task: None,
        started_by: person("Ada"),
        messages,
    }
}

/// Begun with the first batch said: an open discussion about no task yet.
fn begun() -> Discussion {
    let said = vec![
        message("Ada", "09:00:00", "Should the store take a lease?"),
        message("Bob", "09:05:00", "Yes, with a TTL."),
    ];
    begin(opening(said), &when("09:10:00")).unwrap()
}

#[test]
fn a_discussion_begins_with_the_messages_said_so_far() {
    let discussion = begun();
    let fields = discussion.fields();
    assert_eq!(discussion.fields().state, DiscussionState::Open);
    assert_eq!(
        (
            &fields.task,
            &fields.created,
            &fields.updated,
            fields.messages.len()
        ),
        (&None, &at("09:00:00"), &at("09:10:00"), 2)
    );
    let about = task(TaskStatus::Ready);
    let with_task = Opening {
        task: Some(&about),
        ..opening(vec![message("Ada", "09:00:00", "Lease?")])
    };
    let linked = begin(with_task, &when("08:00:00")).unwrap();
    assert_eq!(linked.fields().task, Some(about.fields().id.clone()));
    assert_eq!(
        linked.fields().updated,
        at("09:00:00"),
        "written no earlier than said"
    );
    let none = begin(opening(vec![]), &when("09:00:00"));
    assert!(
        matches!(&none, Err(BoardRuleError::Invalid(error)) if error.field == "messages"),
        "{none:?}"
    );
    let shuffled = begin(
        opening(vec![
            message("Ada", "09:05:00", "b"),
            message("Bob", "09:00:00", "a"),
        ]),
        &when("09:10:00"),
    );
    assert!(
        matches!(&shuffled, Err(BoardRuleError::Invalid(error)) if error.field == "messages/1/at"),
        "{shuffled:?}"
    );
}

/// Messages are added in batches, after the last, and the discussion is
/// written after its last change and no earlier than the batch's last.
#[test]
fn a_batch_of_messages_is_added_to_an_open_discussion() {
    let discussion = begun();
    let batch = vec![
        message("Ada", "09:20:00", "Agreed."),
        message("Carol", "09:30:00", "Ship it."),
    ];
    let added = append(&discussion, batch, &when("09:00:00")).unwrap();
    let fields = added.fields();
    assert_eq!(fields.messages.len(), 4);
    assert_eq!(
        fields.messages[..2],
        discussion.fields().messages[..],
        "earlier messages kept"
    );
    assert_eq!(fields.messages[3].author, person("Carol"));
    assert_eq!(fields.updated, at("09:30:00"));
    let late = append(
        &discussion,
        vec![message("Ada", "09:06:00", "Late.")],
        &when("09:00:00"),
    )
    .unwrap();
    assert_eq!(
        late.fields().updated,
        at("09:10:01"),
        "a second past its last change"
    );
    let ahead = append(
        &discussion,
        vec![message("Ada", "09:07:00", "x")],
        &when("11:00:00"),
    )
    .unwrap();
    assert_eq!(ahead.fields().updated, at("11:00:00"));
    assert_eq!(
        append(&discussion, vec![], &when("10:00:00")),
        Err(BoardRuleError::NoEdit)
    );
    let before = append(
        &discussion,
        vec![message("Ada", "09:04:00", "x")],
        &when("10:00:00"),
    );
    assert!(
        matches!(&before, Err(BoardRuleError::Invalid(error)) if error.field == "messages/2/at"),
        "{before:?}"
    );
}

#[test]
fn a_discussion_is_linked_to_one_task_once() {
    let discussion = begun();
    let about = task(TaskStatus::Ready);
    let linked = link(&discussion, &about, &when("09:00:00")).unwrap();
    assert_eq!(linked.fields().task, Some(about.fields().id.clone()));
    assert_eq!(linked.fields().updated, at("09:10:01"));
    assert_eq!(
        link(&linked, &about, &when("10:00:00")),
        Err(BoardRuleError::NoEdit)
    );
    let mut other = task(TaskStatus::Ready).into_fields();
    other.id = Slug::parse("other").unwrap();
    let other = Task::new(other).unwrap();
    assert_eq!(
        link(&linked, &other, &when("10:00:00")),
        Err(BoardRuleError::DiscussionLinked {
            discussion: id(),
            task: about.fields().id.clone()
        })
    );
    let closed = close(&discussion, &when("10:00:00")).unwrap();
    assert!(
        link(&closed, &about, &when("10:00:00")).is_ok(),
        "a closed discussion is filed later"
    );
}

#[test]
fn a_closed_discussion_takes_no_message_and_is_closed_once() {
    let closed = close(&begun(), &when("09:00:00")).unwrap();
    assert_eq!(closed.fields().state, DiscussionState::Closed);
    assert_eq!(closed.fields().updated, at("09:10:01"));
    let refused = Err(BoardRuleError::DiscussionClosed(id()));
    assert_eq!(
        append(
            &closed,
            vec![message("Bob", "09:20:00", "x")],
            &when("10:00:00")
        ),
        refused
    );
    assert_eq!(close(&closed, &when("10:00:00")), refused);
}

/// A discussion, or a message, a clock running ahead stamped is refused
/// once an outside time bounds the clock, and taken within the slack.
#[test]
fn a_discussion_or_message_stamped_ahead_is_refused() {
    let discussion = begun();
    let ahead = |updated: &str| {
        Err(BoardRuleError::DiscussionAhead {
            discussion: id(),
            updated: at(updated),
        })
    };
    let early = bounded("09:00:00", "09:00:00");
    assert_eq!(
        append(&discussion, vec![message("Ada", "09:00:00", "x")], &early),
        ahead("09:10:00")
    );
    assert_eq!(
        link(&discussion, &task(TaskStatus::Ready), &early),
        ahead("09:10:00")
    );
    assert_eq!(close(&discussion, &early), ahead("09:10:00"));
    let now = bounded("10:00:00", "10:00:00");
    assert_eq!(
        append(&discussion, vec![message("Ada", "10:00:31", "x")], &now),
        ahead("10:00:31")
    );
    let first = opening(vec![message("Ada", "10:00:31", "x")]);
    assert_eq!(begin(first, &now), ahead("10:00:31"));
    assert!(
        append(&discussion, vec![message("Ada", "10:00:30", "x")], &now).is_ok(),
        "within the slack"
    );
    assert!(close(&discussion, &bounded("09:00:00", "09:09:30")).is_ok());
}
