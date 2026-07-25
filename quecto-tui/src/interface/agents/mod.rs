//! Agents presentation policy extracted from the legacy `App` slice (#1222).
//!
//! Runtime glue still lives beside `App`; these modules own the pure roster,
//! feed-synchronization, ledger, and focus state used by that glue.

pub(crate) mod feed;
pub(crate) mod focus;
pub(crate) mod ledger;
pub(crate) mod roster;
pub(crate) mod ui;
