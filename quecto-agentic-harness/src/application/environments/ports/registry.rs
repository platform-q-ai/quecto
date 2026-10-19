//! The durable registry of one base directory's environments.

use crate::application::environments::dto::CorrectionOutcome;
use crate::domain::environments::entities::environment_registry::EnvironmentRecord;
use crate::domain::environments::entities::environment_registry::EnvironmentStatus;

// ─── Durable environments (#2024 S4d) ────────────────────────────────────────

/// The durable registry of one base directory: every session sharing the
/// directory allocates its refs and writes its records here, so a `C*` ref
/// is unique across sessions and outlives the harness that minted it.
/// Members are never stored: they belong to the session that launched
/// them. Every operation is synchronous — a record write is small and must
/// have landed before the launch that committed it returns.
pub trait EnvironmentRegistryStore: Send + Sync {
    /// The next ref number nothing holds — no record on file, no mint still
    /// in flight, nothing below `floor` (the caller's own records the file
    /// may have forgotten) — allocated under an exclusive hold so two
    /// sessions never receive the same one (#2070). The number is held as
    /// in flight until it is recorded, released, or older than a create can
    /// take.
    fn allocate_ref(&self, floor: u64) -> Result<u64, String>;
    /// Give back an allocated number the caller will not record (#2070): a
    /// create that failed after minting. Unknown numbers are ignored.
    fn release_ref(&self, number: u64) -> Result<(), String>;
    /// Every record on file, in ref order.
    fn load(&self) -> Result<Vec<EnvironmentRecord>, String>;
    /// Write `record` under its ref, replacing what was there.
    fn record(&self, record: &EnvironmentRecord) -> Result<(), String>;
    /// Write `record` only if the record on file still has `expected`
    /// status (read and written under the exclusive hold): a restore's
    /// correction must never revert what another session did meanwhile.
    fn correct(
        &self,
        record: &EnvironmentRecord,
        expected: &EnvironmentStatus,
    ) -> Result<CorrectionOutcome, String>;
    /// Remove `record` from the file (a rolled-back create, or a stopped
    /// record whose box left nothing behind, #2134) — only while
    /// its ref still names this environment (#2070): a ref another session
    /// has since taken is left as that session recorded it.
    fn forget(&self, record: &EnvironmentRecord) -> Result<(), String>;
}
