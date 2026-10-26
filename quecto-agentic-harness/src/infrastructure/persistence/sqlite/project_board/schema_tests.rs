//! A revision's chain (epic #2705 B11 review): one pinned vector, so its
//! encoding never changes unseen, and each column it covers changes it,
//! so a column dropped from the hash fails here.
use super::*;

/// A schema-5 board file holding two revisions written by hand, the
/// first's chain made, in a temp dir.
fn two_revisions() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let c = super::super::storage::open(&dir.path().join("board.sqlite3")).unwrap();
    c.execute_batch(
        "INSERT INTO revisions VALUES ('1:1','r1','Ada','ada@example.com',1,NULL,'board: one',100,NULL,NULL,NULL);
         INSERT INTO revisions VALUES ('1:2','r2','Ada','ada@example.com',2,1,'board: two',200,'c0ffee',X'0102',NULL);
         INSERT INTO revision_principals VALUES ('1:2','token:phone');",
    )
    .unwrap();
    let first = chain_of(&c, 1).unwrap();
    c.execute("UPDATE revisions SET chain=?1 WHERE revision=1", [first])
        .unwrap();
    (dir, c)
}

/// The pinned encoding: a change to it changes these, so it is seen. Both
/// were made apart from this code, from the encoding as documented: SHA-256
/// over length-framed fields (u64 big-endian lengths), an absent value one
/// 0 byte and a present one 1 then its bytes.
#[test]
fn the_chain_is_the_pinned_vector() {
    let (_dir, c) = two_revisions();
    assert_eq!(
        chain_of(&c, 1).unwrap(),
        "73eb1b7e00e10fd1ecde174526af201d49ca10ffe56cc0a19f08706bccb4793b",
        "revision 1's chain"
    );
    assert_eq!(
        chain_of(&c, 2).unwrap(),
        "4f136c012e247920e51fa1f929e2acf22d77c9836374eb13fe43cc4297aa1604",
        "revision 2's chain"
    );
}

/// Each column the chain covers changes it: one dropped from the hash
/// leaves its change unseen, and fails here.
#[test]
fn each_column_and_the_chain_before_changes_the_chain() {
    let tampers = [
        ("head", "UPDATE revisions SET head='9:2' WHERE revision=2"),
        ("root", "UPDATE revisions SET root='r9' WHERE revision=2"),
        ("name", "UPDATE revisions SET name='Eve' WHERE revision=2"),
        (
            "email",
            "UPDATE revisions SET email='eve@example.com' WHERE revision=2",
        ),
        (
            "parent",
            "UPDATE revisions SET parent=NULL WHERE revision=2",
        ),
        (
            "message",
            "UPDATE revisions SET message='board: other' WHERE revision=2",
        ),
        ("at", "UPDATE revisions SET at=201 WHERE revision=2"),
        (
            "git_commit",
            "UPDATE revisions SET git_commit=NULL WHERE revision=2",
        ),
        (
            "git_meta",
            "UPDATE revisions SET git_meta=X'0103' WHERE revision=2",
        ),
        (
            "git_meta empty",
            "UPDATE revisions SET git_meta=X'' WHERE revision=2",
        ),
        (
            "principal",
            "UPDATE revision_principals SET principal='local' WHERE head='1:2'",
        ),
        (
            "no principal",
            "DELETE FROM revision_principals WHERE head='1:2'",
        ),
        (
            "the chain before",
            "UPDATE revisions SET chain='forked' WHERE revision=1",
        ),
    ];
    for (column, tamper) in tampers {
        let (_dir, c) = two_revisions();
        let before = chain_of(&c, 2).unwrap();
        c.execute_batch(tamper).unwrap();
        let after = chain_of(&c, 2).unwrap_or_default();
        assert_ne!(after, before, "{column} is not in the chain");
    }
}

#[test]
fn a_revision_whose_chain_before_is_missing_has_none() {
    let (_dir, c) = two_revisions();
    c.execute_batch("UPDATE revisions SET chain=NULL WHERE revision=1")
        .unwrap();
    assert!(chain_of(&c, 2).is_err());
}
