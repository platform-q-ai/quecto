//! UDS delivery of the catalogue capability (#1845): controllers map wire
//! commands onto the use cases composition injected; presenters render
//! their DTOs on the wire. No policy lives here.

pub mod effort_presenter;
pub mod list_models_controller;
pub mod list_models_presenter;
pub mod model_presenter;
