//! Contract for [`BoardTokens`] (epic #2705 B5), run against the file
//! adapter and the in-memory test double alike: a token made once
//! verifies as its name until revoked; names are unique; the listing is
//! names and dates; revoking an unknown name is refused. The SQLite
//! admin-revision adapter (after B2) must pass it too.
use quecto::application::project_board::ports::{BoardTokens, TokenError};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::infrastructure::persistence::file::board_tokens::FileBoardTokens;
use quecto::infrastructure::test_support::memory_board_tokens::MemoryBoardTokens;

fn name(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn contract(tokens: &dyn BoardTokens) {
    let at = Timestamp::parse("2026-10-24T12:00:00Z").unwrap();
    assert!(tokens.list().unwrap().is_empty());
    let laptop = tokens.add(&name("laptop"), &at).unwrap();
    let phone = tokens.add(&name("phone"), &at).unwrap();
    assert_eq!(laptop.record.name, name("laptop"));
    assert_eq!(laptop.record.created, at);
    assert_ne!(laptop.secret, phone.secret);
    assert_eq!(
        tokens.add(&name("laptop"), &at),
        Err(TokenError::Exists(name("laptop")))
    );
    assert_eq!(tokens.verify(&laptop.secret), Ok(Some(name("laptop"))));
    assert_eq!(tokens.verify(&phone.secret), Ok(Some(name("phone"))));
    assert_eq!(tokens.verify("not held"), Ok(None));
    let mut listed: Vec<String> = tokens
        .list()
        .unwrap()
        .iter()
        .map(|token| format!("{} {}", token.name.as_str(), token.created.as_str()))
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        ["laptop 2026-10-24T12:00:00Z", "phone 2026-10-24T12:00:00Z"]
    );
    tokens.revoke(&name("laptop")).unwrap();
    assert_eq!(tokens.verify(&laptop.secret), Ok(None));
    assert_eq!(
        tokens.revoke(&name("laptop")),
        Err(TokenError::NoSuchToken(name("laptop")))
    );
    assert_eq!(tokens.verify(&phone.secret), Ok(Some(name("phone"))));
}

#[test]
fn the_file_adapter_keeps_the_contract() {
    let dir = tempfile::tempdir().unwrap();
    contract(&FileBoardTokens::open(&dir.path().join("state")).unwrap());
}

#[test]
fn the_memory_double_keeps_the_contract() {
    contract(&MemoryBoardTokens::default());
}
