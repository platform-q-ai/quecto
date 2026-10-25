/// The board's head: the store's revision a write must start from (the
/// git store's commit id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoardHead(pub String);
