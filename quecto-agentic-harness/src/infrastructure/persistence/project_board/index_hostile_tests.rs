//! Files planted where the index lives: links, and databases dressed as
//! the index. None is followed, written to or trusted; each ends in a clean
//! rebuild or a refusal.
use super::*;

fn victim(dir: &Path) -> (std::path::PathBuf, Vec<u8>) {
    let path = dir.join("victim.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE precious (x); INSERT INTO precious VALUES ('keep me');")
        .unwrap();
    drop(connection);
    let bytes = std::fs::read(&path).unwrap();
    (path, bytes)
}

/// The schema objects of the file at `path`, as a fresh index has them.
fn objects(path: &Path) -> Vec<(String, String)> {
    let connection = rusqlite::Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT type, name FROM sqlite_master ORDER BY type, name")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn an_index_that_is_a_link_is_never_followed_and_is_replaced() {
    let (dir, index) = index();
    let (victim, bytes) = victim(dir.path());
    std::fs::create_dir(dir.path().join("quecto")).unwrap();
    std::os::unix::fs::symlink(&victim, index.path()).unwrap();
    assert!(needs_rebuild(&index.ready()));
    assert_eq!(
        rebuild(&index, &board("a1")),
        Ok(Refreshed::Rebuilt {
            head: board("a1").head.0
        })
    );
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        bytes,
        "the victim is untouched"
    );
    assert!(
        std::fs::symlink_metadata(index.path())
            .unwrap()
            .file_type()
            .is_file()
    );
    assert_eq!(ids(index.ready()), ["a"]);
    std::fs::remove_file(index.path()).unwrap();
    std::os::unix::fs::symlink(&victim, index.path()).unwrap();
    index.discard().unwrap();
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        bytes,
        "discarding removes the link only"
    );
}

#[test]
fn an_index_directory_that_is_a_link_is_refused() {
    let (dir, index) = index();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.path().join("quecto")).unwrap();
    assert!(matches!(
        rebuild(&index, &board("a1")),
        Err(IndexError::Io(_))
    ));
    assert!(matches!(index.discard(), Err(IndexError::Io(_))));
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "nothing is written through the link"
    );
}

#[test]
fn planted_triggers_views_and_tables_never_survive_a_rebuild() {
    for plant in [
        // A trigger that forges the head on every insert.
        "CREATE TRIGGER forge AFTER INSERT ON tasks BEGIN UPDATE meta SET value = 'x' WHERE key = 'head'; END;",
        // A trigger that blocks every rebuild in place.
        "CREATE TRIGGER block BEFORE DELETE ON tasks BEGIN SELECT RAISE(ABORT, 'no'); END;",
        // A view standing in for the tasks table.
        "DROP TABLE tasks; CREATE VIEW tasks AS SELECT 'a' AS id, 'ready' AS status, NULL AS parent, 0 AS claimed, x'00' AS record;",
        // The right names with the wrong columns, at the right version.
        "DROP TABLE quarantined; CREATE TABLE quarantined (path TEXT PRIMARY KEY, reason TEXT NOT NULL);",
        // An extra table.
        "CREATE TABLE leftover (x);",
    ] {
        let (_dir, index) = index();
        rebuild(&index, &board("a1")).unwrap();
        let fresh = objects(index.path());
        tamper(&index, plant);
        assert!(needs_rebuild(&index.ready()), "{plant}");
        assert!(needs_rebuild(&index.head()), "{plant}");
        assert_eq!(
            rebuild(&index, &board("a1")),
            Ok(Refreshed::Rebuilt {
                head: board("a1").head.0
            }),
            "{plant}"
        );
        assert_eq!(objects(index.path()), fresh, "{plant}");
        assert_eq!(ids(index.ready()), ["a"], "{plant}");
        assert_eq!(index.head(), Ok(Some(board("a1").head.0)), "{plant}");
    }
}

/// A planted set-aside row whose path no store would show (a right-to-left
/// override, say) is refused as corrupt, which the listings answer by
/// discarding the index and rebuilding it, never by showing the row.
#[test]
fn a_planted_path_no_store_shows_ends_in_a_rebuild() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    tamper(
        &index,
        "UPDATE quarantined SET path = 'tasks/a' || char(8238) || 'x.json';",
    );
    let read = index.ready();
    assert!(
        matches!(&read, Err(IndexError::Corrupt(why)) if why.contains("no store shows")),
        "{read:?}"
    );
    // As the listings do with a corrupt index: discard it and rebuild.
    index.discard().unwrap();
    assert_eq!(
        rebuild(&index, &board("a1")),
        Ok(Refreshed::Rebuilt {
            head: board("a1").head.0
        })
    );
    let listed = index.ready().unwrap().quarantined;
    assert!(
        listed.iter().all(|file| shown_path(&file.path)),
        "{listed:?}"
    );
}
