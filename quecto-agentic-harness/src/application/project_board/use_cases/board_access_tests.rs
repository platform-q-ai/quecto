use super::*;
use crate::infrastructure::test_support::memory_board_tokens::MemoryBoardTokens;
use crate::infrastructure::time::board_clock::SystemBoardClock;

fn access() -> (Arc<MemoryBoardTokens>, BoardAccess) {
    let tokens = Arc::new(MemoryBoardTokens::default());
    let access = BoardAccess::new(
        Arc::clone(&tokens) as Arc<dyn BoardTokens>,
        Arc::new(SystemBoardClock::default()),
    );
    (tokens, access)
}

/// B5 red: a local caller needs no token; a remote one is a device only
/// with a held token: none, an unknown one or a revoked one is refused.
#[test]
fn a_remote_caller_is_a_device_only_with_a_held_token() {
    let (_, access) = access();
    assert_eq!(access.authenticate(None), Err(Refused::Token));
    let issued = access.add("laptop").unwrap();
    let name = Slug::parse("laptop").unwrap();
    assert_eq!(access.authenticate(Some(&issued.secret)), Ok(name.clone()));
    assert_eq!(
        access.authenticate(Some("token-phone")),
        Err(Refused::Token)
    );
    assert_eq!(access.revoke("laptop"), Ok(name));
    assert_eq!(
        access.authenticate(Some(&issued.secret)),
        Err(Refused::Token)
    );
}

/// B5 red: reads are for both; admin is for a local caller only.
#[test]
fn admin_is_for_a_local_caller_only() {
    let device = Principal::Device(Slug::parse("laptop").unwrap());
    assert_eq!(
        BoardAccess::authorize(&Principal::Local, Scope::Read),
        Ok(())
    );
    assert_eq!(
        BoardAccess::authorize(&Principal::Local, Scope::Admin),
        Ok(())
    );
    assert_eq!(BoardAccess::authorize(&device, Scope::Read), Ok(()));
    assert_eq!(
        BoardAccess::authorize(&device, Scope::Admin),
        Err(Refused::LocalOnly)
    );
}

#[test]
fn tokens_that_cannot_be_read_refuse_as_unavailable() {
    let (tokens, access) = access();
    tokens.fail_with(TokenError::Unavailable("the disk is gone".into()));
    assert_eq!(
        access.authenticate(Some("anything")),
        Err(Refused::Unavailable(TokenError::Unavailable(
            "the disk is gone".into()
        )))
    );
}

#[test]
fn a_name_that_is_no_slug_is_refused_before_the_store() {
    let (_, access) = access();
    for name in ["", "Laptop", "my laptop", "-x", "a/b", &"x".repeat(65)] {
        let error = access.add(name).unwrap_err();
        assert!(matches!(error, TokenError::BadName(_)), "{name}: {error:?}");
        assert!(matches!(access.revoke(name), Err(TokenError::BadName(_))));
    }
    assert!(access.list().unwrap().is_empty());
}

#[test]
fn add_list_and_revoke_say_names_and_dates() {
    let (_, access) = access();
    let issued = access.add("laptop").unwrap();
    assert!(
        !format!("{issued:?}").contains(&issued.secret),
        "Debug hides it"
    );
    assert_eq!(
        access.add("laptop"),
        Err(TokenError::Exists(Slug::parse("laptop").unwrap()))
    );
    assert_eq!(access.list().unwrap(), vec![issued.record.clone()]);
    assert_eq!(
        access.revoke("phone"),
        Err(TokenError::NoSuchToken(Slug::parse("phone").unwrap()))
    );
}
