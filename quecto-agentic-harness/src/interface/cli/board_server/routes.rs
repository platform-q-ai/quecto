//! Red stub (B4): the board service's route table.
use crate::application::project_board::use_cases::ServeBoard;
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceReply, ServiceRequest,
};

pub struct BoardRoutes {
    serve: ServeBoard,
}

impl BoardRoutes {
    pub fn new(serve: ServeBoard) -> Self {
        Self { serve }
    }
}

impl ServiceHandler for BoardRoutes {
    fn answer(&self, request: &ServiceRequest) -> ServiceReply {
        ServiceReply::json(
            501,
            &serde_json::json!({ "error": format!("{} not yet routed", request.path), "board": self.serve.board().as_str() }),
        )
    }
}

#[cfg(test)]
#[path = "routes_tests.rs"]
pub(super) mod tests;
