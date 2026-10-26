//! Contract for [`AdminRows`] (epic #2705 B11) on the tokens file: a
//! primary gives its tokens' names, dates and digests (never a token); a
//! follower holds the rows it is given alone, so a token revoked there is
//! refused here; rows not as a token is kept are refused whole; the same
//! rows again write nothing.
use std::os::unix::fs::MetadataExt;

use quecto::application::project_board::ports::replication::{AdminRows, AdminToken};
use quecto::application::project_board::ports::{BoardTokens, TokenKind};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::infrastructure::persistence::file::board_tokens::FileBoardTokens;

fn name(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

#[test]
fn rows_are_given_and_taken_whole() {
    let (here, there) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let primary = FileBoardTokens::open(&here.path().join("state")).unwrap();
    let follower = FileBoardTokens::open(&there.path().join("state")).unwrap();
    let at = Timestamp::parse("2026-10-26T09:00:00Z").unwrap();
    let phone = primary.add(&name("phone"), &at).unwrap();
    let laptop = primary.add(&name("laptop"), &at).unwrap();
    let eccup = primary.add_replica(&name("eccup"), &at).unwrap();
    let rows = primary.export().unwrap();
    let eccup_row = rows
        .iter()
        .find(|row| row.name == name("eccup"))
        .unwrap()
        .clone();
    assert_eq!(eccup_row.kind, TokenKind::Replica);
    let rows: Vec<AdminToken> = rows
        .into_iter()
        .filter(|row| row.name != name("eccup"))
        .collect();
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["laptop", "phone"], "by name");
    for row in &rows {
        assert_eq!(row.digest.len(), 64);
        assert!(!row.digest.contains(&phone.secret) && !row.digest.contains(&laptop.secret));
    }
    let mut all = rows.clone();
    all.insert(0, eccup_row);
    follower.replace(&all).unwrap();
    assert_eq!(follower.verify(&phone.secret), Ok(Some(name("phone"))));
    assert_eq!(
        follower.kind_of(&name("eccup")),
        Ok(Some(TokenKind::Replica))
    );
    assert_eq!(follower.verify(&eccup.secret), Ok(Some(name("eccup"))));
    assert_eq!(follower.export().unwrap(), all);
    let kept: Vec<AdminToken> = rows
        .iter()
        .filter(|row| row.name == name("phone"))
        .cloned()
        .collect();
    follower.replace(&kept).unwrap();
    assert_eq!(
        follower.verify(&laptop.secret),
        Ok(None),
        "revoked by omission"
    );
    assert_eq!(follower.verify(&phone.secret), Ok(Some(name("phone"))));
}

#[test]
fn rows_not_as_kept_are_refused_and_the_same_rows_write_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let tokens = FileBoardTokens::open(&state).unwrap();
    let at = Timestamp::parse("2026-10-26T09:00:00Z").unwrap();
    let phone = tokens.add(&name("phone"), &at).unwrap();
    let rows = tokens.export().unwrap();
    let stamp = || {
        let meta = std::fs::metadata(state.join("tokens.json")).unwrap();
        (meta.ino(), meta.mtime(), meta.mtime_nsec())
    };
    let before = stamp();
    tokens.replace(&rows).unwrap();
    assert_eq!(stamp(), before, "the same rows write nothing");
    let mut unshaped = rows.clone();
    unshaped[0].digest = "not a digest".into();
    assert!(tokens.replace(&unshaped).is_err());
    let twice = vec![rows[0].clone(), rows[0].clone()];
    assert!(tokens.replace(&twice).is_err());
    assert_eq!(tokens.export().unwrap(), rows, "a refusal changes nothing");
    assert_eq!(tokens.verify(&phone.secret), Ok(Some(name("phone"))));
}
