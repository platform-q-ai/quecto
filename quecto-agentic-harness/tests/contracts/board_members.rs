//! `BoardMembers` on the SQLite adapter (#2270): member rows in store
//! order, the admitted count, the not-dead count `create` uses and the
//! #1969 claim counts, each by Python's SQL; loosely typed values bound as
//! Python's `sqlite3` binds them and read back as stored (epic P3).
use quecto::application::swarm::dto::{
    BoardLocation, LaunchIdentity, MemberClaimCounts, MemberRow, NewMember,
};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::{BoardError, MemberState, RefusalKind};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::{Value, json};

fn within(
    repository: &SqliteBoardRepository,
    create: bool,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) -> Result<(), BoardError> {
    repository.atomic(create, &mut |transaction| work(transaction))
}

fn new_member(id: &str, status: MemberState) -> NewMember {
    NewMember {
        id: id.into(),
        reservation: format!("{id}-r"),
        status,
        pid: json!(7),
        started: json!("t"),
        socket: Value::Null,
        launcher: Some("parent".into()),
    }
}

#[test]
fn members_are_written_read_and_counted() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    within(&repository, true, |transaction| {
        for (id, status) in [
            ("parent", MemberState::LIVE),
            ("worker", MemberState::RESERVED),
            ("gone", MemberState::DEAD),
        ] {
            transaction.insert_member(&new_member(id, status))?;
        }
        Ok(())
    })
    .unwrap();
    // A status the board never writes is read as it is.
    let raw = |sql: &str| {
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute(sql, [])
            .unwrap();
    };
    raw("INSERT INTO members(id,status) VALUES('odd','paused')");
    within(&repository, false, |transaction| {
        let worker = transaction.member("worker")?.unwrap();
        assert_eq!(worker.status, Some(MemberState::RESERVED));
        assert_eq!(worker.reservation.as_deref(), Some("worker-r"));
        assert!(transaction.member("stranger")?.is_none());
        let members = transaction.members()?;
        assert_eq!(
            members
                .iter()
                .map(|row| row.text("id").unwrap())
                .collect::<Vec<_>>(),
            ["parent", "worker", "gone", "odd"]
        );
        // `dict(row)`: every column, in table order, as stored.
        assert_eq!(
            members[0],
            MemberRow {
                columns: [
                    ("id", json!("parent")),
                    ("reservation", json!("parent-r")),
                    ("status", json!("live")),
                    ("pid", json!(7)),
                    ("started", json!("t")),
                    ("socket", json!(null)),
                    ("launcher", json!("parent")),
                ]
                .map(|(name, value)| (name.to_owned(), value))
                .to_vec(),
            }
        );
        Ok(())
    })
    .unwrap();
    // No status at all: counted by neither rule, and listed as `None`.
    raw("INSERT INTO members(id,status) VALUES('blank',NULL)");
    within(&repository, false, |transaction| {
        assert_eq!(transaction.usage()?, 2, "live and reserved");
        assert_eq!(
            transaction.not_dead()?,
            3,
            "status != 'dead': NULL is not counted"
        );
        assert_eq!(
            transaction.claim_counts(Some("parent"))?,
            MemberClaimCounts {
                members_without_claim: 1,
                members_dead: 1
            }
        );
        let blank = transaction.members()?.pop().unwrap();
        assert_eq!(
            (blank.text("id"), blank.get("status")),
            (Some("blank"), Some(&json!(null)))
        );
        assert_eq!(transaction.member("blank")?.unwrap().status, None);
        Ok(())
    })
    .unwrap();
    // Columns added outside the board are listed after the board's, and a
    // NULL id is listed as it is (#2270 round-3 review N5).
    raw("ALTER TABLE members ADD COLUMN extra REAL");
    raw("INSERT INTO members(id,status,extra) VALUES(NULL,'live',2.5)");
    within(&repository, false, |transaction| {
        let added = transaction.members()?.pop().unwrap();
        let names: Vec<&str> = added
            .columns
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "id",
                "reservation",
                "status",
                "pid",
                "started",
                "socket",
                "launcher",
                "extra"
            ]
        );
        assert_eq!(added.get("id"), Some(&json!(null)));
        assert_eq!(added.get("extra"), Some(&json!(2.5)));
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_second_row_for_one_member_is_a_store_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    });
    let refused = within(&repository, true, |transaction| {
        transaction.insert_member(&new_member("parent", MemberState::LIVE))?;
        transaction.insert_member(&NewMember {
            reservation: "another".into(),
            ..new_member("parent", MemberState::LIVE)
        })
    })
    .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(
            RefusalKind::Store,
            "coordination store unavailable or contended: UNIQUE constraint failed: members.id"
        )
    );
}

/// P3: each JSON value binds as Python's `sqlite3` binds it and the column
/// affinity decides what is kept; a pid reads back in whatever storage
/// class it holds. A value Python cannot bind is a store refusal naming the
/// parameter.
#[test]
fn loose_values_bind_as_python_binds_them_and_read_as_stored() {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    });
    let cases = [
        (
            json!(true),
            json!(5),
            json!(null),
            json!(1),
            Some("5"),
            None,
        ),
        (
            json!("12"),
            json!(1.5),
            json!(false),
            json!(12),
            Some("1.5"),
            Some("0"),
        ),
        (
            json!(3.5),
            json!(""),
            json!("s"),
            json!(3.5),
            Some(""),
            Some("s"),
        ),
        (
            json!("abc"),
            json!(null),
            json!(-2),
            json!("abc"),
            None,
            Some("-2"),
        ),
        (json!(3.0), json!(null), json!(null), json!(3), None, None),
    ];
    for (index, (pid, started, socket, read_pid, read_started, read_socket)) in
        cases.into_iter().enumerate()
    {
        let id = format!("m{index}");
        within(&repository, true, |transaction| {
            transaction.insert_member(&NewMember {
                pid: pid.clone(),
                started: started.clone(),
                socket: socket.clone(),
                launcher: None,
                ..new_member(&id, MemberState::LIVE)
            })
        })
        .unwrap();
        within(&repository, false, |transaction| {
            let row = transaction
                .members()?
                .into_iter()
                .find(|row| row.text("id") == Some(id.as_str()))
                .unwrap();
            assert_eq!(row.get("pid"), Some(&read_pid), "{pid}");
            assert_eq!(row.text("started"), read_started, "{started}");
            assert_eq!(row.text("socket"), read_socket, "{socket}");
            Ok(())
        })
        .unwrap();
    }
    // Refusals name the pid's position in Python's `_bootstrap` statement
    // (`VALUES(?,?,'live',?,?,?)`: parameter 3), as Python's do.
    for (pid, message) in [
        (
            json!([1]),
            "Error binding parameter 3: type 'list' is not supported",
        ),
        (
            json!({"a": 1}),
            "Error binding parameter 3: type 'dict' is not supported",
        ),
        (
            json!(u64::MAX),
            "Error binding parameter 3: Python int too large to convert to SQLite INTEGER",
        ),
    ] {
        let refused = within(&repository, false, |transaction| {
            transaction.insert_member(&NewMember {
                pid: pid.clone(),
                ..new_member("refused", MemberState::LIVE)
            })
        })
        .unwrap_err();
        assert_eq!(
            refused,
            BoardError::new(
                RefusalKind::Invalid,
                format!("coordination store unavailable or contended: {message}")
            ),
            "{pid}"
        );
    }
}

fn repository_in(dir: &tempfile::TempDir) -> SqliteBoardRepository {
    SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    })
}

fn row(columns: [(&str, Value); 7]) -> MemberRow {
    MemberRow {
        columns: columns
            .map(|(name, value)| (name.to_owned(), value))
            .to_vec(),
    }
}

/// #2271: admission, activation and the launch records, each by Python's
/// SQL. `reserve_member` writes a process-less `reserved` row launched by
/// the given actor; `member_row` answers `dict(row)` by id, or by id and
/// reservation; the updates touch only their columns.
#[test]
fn launch_records_are_written_by_pythons_sql() {
    let dir = tempfile::tempdir().unwrap();
    let repository = repository_in(&dir);
    let launch = LaunchIdentity {
        pid: json!(4242),
        started: json!("Mon Sep 28"),
    };
    within(&repository, true, |transaction| {
        transaction.insert_member(&new_member("parent", MemberState::LIVE))?;
        transaction.reserve_member("worker", &json!("worker-r"), "parent")?;
        assert_eq!(
            transaction.member_row(&json!("worker"), None)?,
            Some(row([
                ("id", json!("worker")),
                ("reservation", json!("worker-r")),
                ("status", json!("reserved")),
                ("pid", json!(null)),
                ("started", json!(null)),
                ("socket", json!(null)),
                ("launcher", json!("parent")),
            ]))
        );
        assert!(
            transaction
                .member_row(&json!("worker"), Some(&json!("other")))?
                .is_none()
        );
        assert!(transaction.member_row(&json!("stranger"), None)?.is_none());
        assert_eq!(
            transaction
                .member_row(&json!("worker"), Some(&json!("worker-r")))?
                .and_then(|row| row.text("id").map(str::to_owned)),
            Some("worker".to_owned())
        );
        transaction.record_launch(&json!("worker"), &launch)?;
        let recorded = transaction.member_row(&json!("worker"), None)?.unwrap();
        assert_eq!(
            (
                recorded.get("pid"),
                recorded.text("started"),
                recorded.text("status")
            ),
            (Some(&json!(4242)), Some("Mon Sep 28"), Some("reserved"))
        );
        transaction.activate_member(&json!("worker"), &launch, &json!("/w.sock"))?;
        let active = transaction.member_row(&json!("worker"), None)?.unwrap();
        assert_eq!(
            (active.text("status"), active.text("socket")),
            (Some("live"), Some("/w.sock"))
        );
        transaction.activate_member(&json!("worker"), &launch, &Value::Null)?;
        assert_eq!(
            transaction
                .member_row(&json!("worker"), None)?
                .unwrap()
                .get("socket"),
            Some(&json!(null))
        );
        transaction.set_socket("worker", &json!("/again.sock"))?;
        assert_eq!(
            transaction
                .member_row(&json!("worker"), None)?
                .unwrap()
                .text("socket"),
            Some("/again.sock")
        );
        // An unknown member is left as it is: nothing to update, no error.
        transaction.set_socket("stranger", &json!("/s"))?;
        assert!(transaction.member_row(&json!("stranger"), None)?.is_none());
        transaction.reserve_member("unlaunched", &json!("u-r"), "worker")?;
        transaction.mark_member_dead_unlaunched(&json!("unlaunched"))?;
        let dead = transaction.member_row(&json!("unlaunched"), None)?.unwrap();
        assert_eq!(
            (dead.text("status"), dead.text("launcher")),
            (Some("dead"), Some("worker"))
        );
        assert_eq!(transaction.usage()?, 2, "parent and worker");
        Ok(())
    })
    .unwrap();
}

/// #2271 round-1 review M1: the membership statements bind the caller's
/// values as Python's `sqlite3` does. The column affinity stores them (an
/// INTEGER pid takes numeric text, a TEXT column a number's text), `id=?`
/// finds `'5'` by `5`, `reservation=?` finds nothing by NULL, and a value
/// `sqlite3` cannot bind is refused naming its position in the statement.
#[test]
fn membership_values_bind_as_pythons_sqlite3_binds_them() {
    let dir = tempfile::tempdir().unwrap();
    let repository = repository_in(&dir);
    within(&repository, true, |transaction| {
        transaction.reserve_member("5", &json!(5), "parent")?;
        let row = transaction.member_row(&json!(5), None)?.unwrap();
        assert_eq!(row.get("reservation"), Some(&json!("5")));
        assert!(
            transaction
                .member_row(&json!("5"), Some(&json!(5)))?
                .is_some()
        );
        assert!(
            transaction
                .member_row(&json!(5), Some(&Value::Null))?
                .is_none()
        );
        assert!(transaction.member_row(&Value::Null, None)?.is_none());
        let launch = LaunchIdentity {
            pid: json!("7"),
            started: json!(5.5),
        };
        transaction.record_launch(&json!(5), &launch)?;
        transaction.activate_member(&json!(5.0), &launch, &json!(true))?;
        let row = transaction.member_row(&json!("5"), None)?.unwrap();
        assert_eq!(
            (row.get("pid"), row.get("started"), row.get("status")),
            (
                Some(&json!(7)),
                Some(&json!("5.5")),
                Some(&json!("reserved"))
            ),
            "5.0 is the text '5.0' under TEXT affinity: no row was activated"
        );
        transaction.activate_member(&json!(5), &launch, &json!(true))?;
        transaction.set_socket("5", &json!(false))?;
        let row = transaction.member_row(&json!("5"), None)?.unwrap();
        assert_eq!(
            (row.get("status"), row.get("socket")),
            (Some(&json!("live")), Some(&json!("0")))
        );
        Ok(())
    })
    .unwrap();
    let list = |position: usize| {
        BoardError::new(
            RefusalKind::Invalid,
            format!(
                "coordination store unavailable or contended: \
             Error binding parameter {position}: type 'list' is not supported"
            ),
        )
    };
    let launch = |pid: Value, started: Value| LaunchIdentity { pid, started };
    for (position, refused) in [
        (
            1,
            within(&repository, false, |transaction| {
                transaction.member_row(&json!([5]), None).map(drop)
            }),
        ),
        (
            2,
            within(&repository, false, |transaction| {
                transaction
                    .member_row(&json!("5"), Some(&json!(["5"])))
                    .map(drop)
            }),
        ),
        (
            2,
            within(&repository, false, |transaction| {
                transaction.reserve_member("6", &json!([6]), "parent")
            }),
        ),
        (
            3,
            within(&repository, false, |transaction| {
                transaction.activate_member(&json!("5"), &launch(json!(1), json!("s")), &json!([]))
            }),
        ),
        (
            4,
            within(&repository, false, |transaction| {
                transaction.activate_member(
                    &json!(["5"]),
                    &launch(json!(1), json!("s")),
                    &Value::Null,
                )
            }),
        ),
        (
            2,
            within(&repository, false, |transaction| {
                transaction.record_launch(&json!("5"), &launch(json!(1), json!([])))
            }),
        ),
        (
            1,
            within(&repository, false, |transaction| {
                transaction.mark_member_dead_unlaunched(&json!([]))
            }),
        ),
        (
            1,
            within(&repository, false, |transaction| {
                transaction.set_socket("5", &json!([]))
            }),
        ),
    ] {
        assert_eq!(refused, Err(list(position)));
    }
}

/// A reservation is unique, as the schema declares, and a second row for
/// one member is refused: both are store refusals with SQLite's text.
#[test]
fn a_reused_reservation_or_identity_is_a_store_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let repository = repository_in(&dir);
    within(&repository, true, |transaction| {
        transaction.reserve_member("worker", &json!("r"), "parent")
    })
    .unwrap();
    for (member, reservation, column) in [
        ("other", "r", "members.reservation"),
        ("worker", "s", "members.id"),
    ] {
        let refused = within(&repository, false, |transaction| {
            transaction.reserve_member(member, &json!(reservation), "parent")
        })
        .unwrap_err();
        assert_eq!(
            refused,
            BoardError::new(
                RefusalKind::Store,
                format!(
                    "coordination store unavailable or contended: UNIQUE constraint failed: {column}"
                )
            )
        );
    }
}

/// #2273: `lost_members` is Python's one ordered scan of `scope_unknown`
/// and `activated` events: a member is lost when its latest
/// `scope_unknown` is newer than its latest `activated`, and answered in
/// the order asked.
#[test]
fn lost_members_are_those_quarantined_after_their_latest_activation() {
    let dir = tempfile::tempdir().unwrap();
    let repository = repository_in(&dir);
    within(&repository, true, |transaction| {
        let events = [
            ("activated", json!({"member": "a"})),
            ("scope_unknown", json!({"member": "a"})),
            ("scope_unknown", json!({"member": "b"})),
            ("activated", json!({"member": "b"})),
            ("scope_unknown", json!({"member": "c"})),
            ("scope_observed", json!({"member": "d"})),
            ("scope_unknown", json!({"member": 5})),
            ("scope_unknown", json!(["e"])),
        ];
        for (action, detail) in events {
            transaction.event("x", 1.0, action, &detail)?;
        }
        assert_eq!(
            transaction.lost_members(&["c", "b", "a", "d", "5"])?,
            ["c", "a"]
        );
        assert!(transaction.lost_members(&[])?.is_empty());
        Ok(())
    })
    .unwrap();
}

/// #2275 (PR #2321 review): `member_status` is Python's `SELECT status
/// FROM members WHERE id=?`. Only the status is read, so another column
/// holding what the board never writes (text that is not UTF-8) does not
/// refuse it. The status is its text, or none for NULL or bytes (TEXT
/// affinity keeps no number); a missing row (or a NULL id) is none; the
/// id binds as Python's `sqlite3` binds it; a status that is not UTF-8 is
/// Python's refusal.
#[test]
fn a_member_status_is_read_alone() {
    let dir = tempfile::tempdir().unwrap();
    let repository = repository_in(&dir);
    within(&repository, true, |transaction| {
        for id in ["5", "live", "blob", "null", "bad"] {
            transaction.reserve_member(id, &json!(format!("{id}-r")), "parent")?;
        }
        Ok(())
    })
    .unwrap();
    rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
        .unwrap()
        .execute_batch(
            "UPDATE members SET socket=CAST(X'FF' AS TEXT);
             UPDATE members SET status='live' WHERE id='live';
             UPDATE members SET status=CAST('live' AS BLOB) WHERE id='blob';
             UPDATE members SET status=NULL WHERE id='null';
             UPDATE members SET status=CAST(X'FF' AS TEXT) WHERE id='bad';",
        )
        .unwrap();
    within(&repository, false, |transaction| {
        let status = |id: Value| {
            transaction
                .member_status(&id)
                .map(|found| found.map(|row| row.status))
        };
        assert_eq!(status(json!(5))?, Some(Some("reserved".to_owned())));
        assert_eq!(status(json!("live"))?, Some(Some("live".to_owned())));
        for id in ["blob", "null"] {
            assert_eq!(status(json!(id))?, Some(None), "{id}");
        }
        assert_eq!(status(json!("stranger"))?, None);
        assert_eq!(status(Value::Null)?, None);
        let undecodable = status(json!("bad")).unwrap_err();
        assert!(
            undecodable
                .message()
                .contains("Could not decode to UTF-8 column 'status'"),
            "{undecodable:?}"
        );
        let unbindable = status(json!([5])).unwrap_err();
        assert!(
            unbindable.message().contains("Error binding parameter 1"),
            "{unbindable:?}"
        );
        Ok(())
    })
    .unwrap();
}
