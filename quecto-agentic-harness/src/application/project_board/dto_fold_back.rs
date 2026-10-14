//! What a fold-back comes to (S4): its result, and what it saw, for its
//! `board_fold_back` record.
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::value_objects::telemetry::{
    FoldBackRecord, ItemCounts, RunOutcomeKind,
};

/// What came of a fold-back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldedBack {
    /// The run as the board now holds it.
    Folded(Run),
    /// The run (by its number) had ended already: nothing was written.
    Ended(u32),
}

/// What a fold-back saw (S4), for its `board_fold_back` record: gathered
/// only for a recorded fold-back (one given none notes nothing), and only
/// ever kinds and counts. Each fact is `None` where the fold-back did not
/// get that far, or (a run already ended) had nothing to fold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldBackFacts {
    pub run_outcome: Option<RunOutcomeKind>,
    pub items: Option<ItemCounts>,
    pub unmatched: Option<u64>,
    pub extra: Option<u64>,
    pub evidence: Option<u64>,
    pub evidence_dropped: Option<u64>,
    pub cost_tokens: Option<u64>,
    /// The write's attempts, and how many lost the race.
    pub attempts: Option<u64>,
    pub lost: Option<u64>,
    /// The UTF-8 bytes of the folded run's text.
    pub bytes: Option<u64>,
}

impl FoldBackFacts {
    /// Writes each fact there is into `record`, leaving every field it has
    /// none for as it was.
    pub fn fill(&self, record: &mut FoldBackRecord) {
        let FoldBackRecord {
            task: _,
            run: _,
            outcome: _,
            run_outcome,
            items,
            unmatched,
            extra,
            evidence,
            evidence_dropped,
            cost_tokens,
            attempts,
            lost,
            bytes,
            duration_us: _,
        } = record;
        let facts = [
            (unmatched, self.unmatched),
            (extra, self.extra),
            (evidence, self.evidence),
            (evidence_dropped, self.evidence_dropped),
            (cost_tokens, self.cost_tokens),
            (attempts, self.attempts),
            (lost, self.lost),
            (bytes, self.bytes),
        ];
        for (field, fact) in facts {
            *field = fact.or(*field);
        }
        *run_outcome = self.run_outcome.or(*run_outcome);
        *items = self.items.or(*items);
    }
}
