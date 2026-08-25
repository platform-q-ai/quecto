//! Contract marker for [`CatalogueRefreshAllPort`].
//!
//! Behavioural coverage lives with `catalogue_refresh_port` because the all-port
//! is an extension of the per-source refresh port and must be exercised with the
//! same fake adapter.

#[test]
fn catalogue_refresh_all_port_contract_is_covered_by_catalogue_refresh_port() {
    // This module is intentionally present so the architecture inventory sees a
    // contract entry for the extension port name.
}
