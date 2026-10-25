use super::*;
use crate::domain::project_board::value_objects::telemetry::BoardId;
struct Port {
    head: BoardHead,
    moved: BoardMoved,
    result: Result<FenceReport, FenceError>,
}
impl BoardFence for Port {
    fn publish_fence(
        &self,
        captured: &BoardHead,
        moved: &BoardMoved,
    ) -> Result<FenceReport, FenceError> {
        assert_eq!(captured, &self.head);
        assert_eq!(moved, &self.moved);
        self.result.clone()
    }
}
#[test]
fn fence_use_case_preserves_capture_order_and_every_typed_publication_outcome() {
    let head = BoardHead("a".repeat(40));
    let moved = BoardMoved::new(
        BoardId::of_hex("0123456789abcdef").unwrap(),
        vec![
            "https://primary.example.test".into(),
            "http://standby.example.test".into(),
        ],
    )
    .unwrap();
    for result in [
        Ok(FenceReport::Already { head: head.clone() }),
        Ok(FenceReport::Published {
            previous: head.clone(),
            head: BoardHead("b".repeat(40)),
        }),
        Err(FenceError::Conflict),
        Err(FenceError::Refused("policy".into())),
        Err(FenceError::Uncertain("lost answer".into())),
        Err(FenceError::Store(
            crate::application::project_board::ports::BoardStoreError::Moved(moved.clone()),
        )),
    ] {
        let operation = FenceBoard::new(Arc::new(Port {
            head: head.clone(),
            moved: moved.clone(),
            result: result.clone(),
        }));
        assert_eq!(operation.execute(&head, &moved), result);
    }
}
