use super::*;
use crate::domain::project_board::entities::discussion::{
    MAX_DISCUSSION_LINES, MAX_MESSAGE_LINE_CHARS, MAX_MESSAGE_LINES, MAX_MESSAGES,
};
use crate::domain::project_board::services::fixtures_tests::{at, person, task};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    DiscussionLimit, DiscussionState, TaskStatus,
};

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
    let linked = begin(with_task, &when("08:59:45")).unwrap();
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
    let added = append(&discussion, batch, &when("09:29:45")).unwrap();
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
        &when("09:05:45"),
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
        matches!(&before, Err(BoardRuleError::Invalid(error)) if error.field == "batch/0/at"),
        "a batch's message is named by its place in the batch: {before:?}"
    );
    let second = append(
        &discussion,
        vec![
            message("Ada", "09:20:00", "ok"),
            message("Bob", "09:21:00", " "),
        ],
        &when("10:00:00"),
    );
    assert!(
        matches!(&second, Err(BoardRuleError::Invalid(error)) if error.field == "batch/1/text/0"),
        "{second:?}"
    );
}

/// A batch that would take a discussion past its most messages, lines or
/// text is refused as full, naming which; a message out of shape is named
/// as such first, never read as the discussion being full.
#[test]
fn a_batch_that_would_overfill_a_discussion_is_refused_as_full() {
    let discussion = begun();
    let full = |limit: DiscussionLimit| {
        Err(BoardRuleError::DiscussionFull {
            discussion: id(),
            limit,
        })
    };
    let one = |text: &str| message("Ada", "09:20:00", text);
    let at_most = vec![one("x"); MAX_MESSAGES - 2];
    assert!(
        append(&discussion, at_most.clone(), &when("10:00:00")).is_ok(),
        "exactly the most"
    );
    let past = [at_most, vec![one("x")]].concat();
    assert_eq!(
        append(&discussion, past, &when("10:00:00")),
        full(DiscussionLimit::Messages)
    );
    let long = vec!["x"; MAX_MESSAGE_LINES].join("\n");
    let many_lines = vec![one(&long); MAX_DISCUSSION_LINES / MAX_MESSAGE_LINES];
    assert_eq!(
        append(&discussion, many_lines.clone(), &when("10:00:00")),
        full(DiscussionLimit::Lines)
    );
    let wide = vec!["|".repeat(MAX_MESSAGE_LINE_CHARS); MAX_MESSAGE_LINES].join("\n");
    let many_bytes = vec![one(&wide); 9];
    assert_eq!(
        append(&discussion, many_bytes, &when("10:00:00")),
        full(DiscussionLimit::Text)
    );
    for (bad, field) in [
        (
            vec!["x"; MAX_MESSAGE_LINES + 1].join("\n"),
            "batch/250/text",
        ),
        ("x".repeat(MAX_MESSAGE_LINE_CHARS + 1), "batch/250/text/0"),
        (" ".to_string(), "batch/250/text/0"),
    ] {
        let mut overfull = many_lines.clone();
        overfull.push(one(&bad));
        let refused = append(&discussion, overfull, &when("10:00:00"));
        assert!(
            matches!(&refused, Err(BoardRuleError::Invalid(error)) if error.field == field),
            "{field}: {refused:?}"
        );
    }
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
    let message_ahead = |stamp: &str| {
        Err(BoardRuleError::MessageAhead {
            discussion: id(),
            at: at(stamp),
        })
    };
    let now = bounded("10:00:00", "10:00:00");
    assert_eq!(
        append(&discussion, vec![message("Ada", "10:00:31", "x")], &now),
        message_ahead("10:00:31")
    );
    let first = opening(vec![message("Ada", "10:00:31", "x")]);
    assert_eq!(begin(first, &now), message_ahead("10:00:31"));
    assert!(
        append(&discussion, vec![message("Ada", "10:00:30", "x")], &now).is_ok(),
        "within the slack"
    );
    assert!(close(&discussion, &bounded("09:00:00", "09:09:30")).is_ok());
}

/// With no outside time, a message is still bounded by the caller's own:
/// one stamped far ahead would leave no later time for any other.
#[test]
fn a_message_from_far_ahead_is_refused_without_an_outside_time() {
    let discussion = begun();
    let far = Timestamp::parse("9999-12-31T23:59:59Z").unwrap();
    let from_far = Message {
        at: far.clone(),
        ..message("Ada", "09:00:00", "x")
    };
    let refused = Err(BoardRuleError::MessageAhead {
        discussion: id(),
        at: far,
    });
    assert_eq!(
        append(&discussion, vec![from_far.clone()], &when("10:00:00")),
        refused
    );
    assert_eq!(begin(opening(vec![from_far]), &when("10:00:00")), refused);
    let near = message("Ada", "10:00:31", "x");
    assert_eq!(
        append(&discussion, vec![near], &when("10:00:00")),
        Err(BoardRuleError::MessageAhead {
            discussion: id(),
            at: at("10:00:31")
        })
    );
    let slack = message("Ada", "10:00:30", "x");
    let taken = append(&discussion, vec![slack], &when("10:00:00")).unwrap();
    assert_eq!(taken.fields().updated, at("10:00:30"));
}

/// A batch past the most messages is refused by its count before any of
/// its messages is read: one out of shape or stamped ahead past the cap is
/// never reached.
#[test]
fn a_batch_past_the_most_messages_is_refused_before_its_messages_are_read() {
    let discussion = begun();
    let mut batch = vec![message("Ada", "09:20:00", "x"); MAX_MESSAGES - 2];
    batch.push(message("Ada", "09:20:00", " "));
    batch.push(message("Ada", "11:00:00", "stamped ahead too"));
    assert_eq!(
        append(&discussion, batch, &when("10:00:00")),
        Err(BoardRuleError::DiscussionFull {
            discussion: id(),
            limit: DiscussionLimit::Messages
        })
    );
}
