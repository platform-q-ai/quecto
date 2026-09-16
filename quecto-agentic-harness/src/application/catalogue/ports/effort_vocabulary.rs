//! The capability fact effort selection depends on (#1848, #1996): which
//! levels the active model accepts. Answered from the published catalogue
//! snapshot by infrastructure; the use case never re-derives a vocabulary
//! from a model name.

use crate::domain::provider::EffortLevel;

pub trait EffortVocabularySource: Send + Sync {
    /// The ordered vocabulary of `model` (a qualified `provider/model` id),
    /// or `None` when the catalogue does not know the model. An empty
    /// vocabulary means the model is known and offers no effort control.
    fn effort_vocabulary(&self, model: &str) -> Option<Vec<EffortLevel>>;
}
