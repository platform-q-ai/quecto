//! The session state a model change acts on (#1847): the model the agent
//! loop sends every `ChatRequest` to, with the per-model limits it clamps
//! to. The loop implements it; the use case is the only writer.

use crate::application::catalogue::dto::ModelLimits;
use crate::application::catalogue::ports::EffortRuntime;

pub trait ModelRuntime: EffortRuntime {
    /// The active model, as a `provider/model` or bare id.
    fn model(&self) -> &str;
    /// Switch model and limits together, from the next turn on.
    fn apply_model(&mut self, model: String, limits: ModelLimits);
}
