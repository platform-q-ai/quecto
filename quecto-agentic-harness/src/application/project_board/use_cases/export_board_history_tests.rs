use super::*;
use crate::application::project_board::ports::BoardHead;
struct Read;
impl BoardHistoryRead for Read {
    fn snapshot(&self) -> HistoryResult<Box<dyn BoardHistorySnapshot>> {
        panic!("proof must use supplied uncommitted view")
    }
}
struct Original;
impl BoardHistorySnapshot for Original {
    fn algorithm(&self) -> HistoryResult<Option<HistoryAlgorithm>> {
        Ok(Some(HistoryAlgorithm::Sha1))
    }
    fn is_original(&self, _: &str) -> HistoryResult<bool> {
        panic!("format before membership")
    }
    fn frontier(&self) -> HistoryResult<BoardHead> {
        Ok(BoardHead("1:1".into()))
    }
    fn next_revision(&self, _: u64) -> HistoryResult<Option<HistoryRevision>> {
        panic!("format before cursor")
    }
    fn object(&self, _: &str) -> HistoryResult<HistoryObject> {
        panic!("format before objects")
    }
}
struct Destination(Option<HistoryAlgorithm>);
impl BoardHistorySink for Destination {
    fn algorithm(&self) -> HistoryResult<Option<HistoryAlgorithm>> {
        Ok(self.0)
    }
    fn raw(&self, _: HistoryKind, _: &[u8], _: Option<&str>, _: bool) -> HistoryResult<String> {
        panic!("format before raw object calls")
    }
    fn tree(&self, _: &[HistoryEntry], _: bool) -> HistoryResult<String> {
        panic!("format before tree calls")
    }
    fn commit(&self, _: HistoryCommit<'_>, _: bool) -> HistoryResult<String> {
        panic!("format before commit calls")
    }
    fn publish(&self, _: Option<&str>, _: &str) -> HistoryResult<()> {
        panic!("no publication")
    }
    fn finish_dry_run(&self) -> HistoryResult<()> {
        panic!("not success")
    }
}
#[test]
fn imported_format_requires_exact_some_before_objects_or_cursor() {
    for format in [None, Some(HistoryAlgorithm::Sha256)] {
        let export = ExportBoardHistory::new(Arc::new(Read), Arc::new(Destination(format)));
        assert!(matches!(
            export.execute_snapshot(&Original, true, None),
            Err(HistoryError::Unsupported(_))
        ));
    }
}
