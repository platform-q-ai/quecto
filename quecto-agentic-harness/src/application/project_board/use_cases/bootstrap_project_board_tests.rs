use super::BootstrapProjectBoard;
use crate::application::project_board::memory_store::MemoryBoardStore;
use crate::application::project_board::ports::{BoardHead, Bootstrapped};
use std::sync::Arc;

#[test]
fn the_board_is_created_once_and_found_after() {
    let store = MemoryBoardStore::default();
    let bootstrap = BootstrapProjectBoard::new(Arc::new(store.client()));
    let created = bootstrap.execute().unwrap();
    assert_eq!(created, Bootstrapped::Created(BoardHead("r1".into())));
    assert_eq!(
        BootstrapProjectBoard::new(Arc::new(store))
            .execute()
            .unwrap(),
        Bootstrapped::Existing(BoardHead("r1".into()))
    );
}
