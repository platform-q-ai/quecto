//! Controller of the fieldless `list_models` command (#1845): asks the
//! injected use case for the listing and hands the outcome to the presenter.
use std::sync::Arc;

use crate::application::catalogue::dto::ModelListingOutcome;
use crate::application::catalogue::use_cases::ListModels;

pub struct ListModelsController {
    list_models: Arc<ListModels>,
}

impl ListModelsController {
    pub fn new(list_models: Arc<ListModels>) -> Self {
        Self { list_models }
    }

    pub fn list(&self) -> ModelListingOutcome {
        self.list_models.execute()
    }
}

impl std::fmt::Debug for ListModelsController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListModelsController")
            .finish_non_exhaustive()
    }
}
