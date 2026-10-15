//! Dispatch shape uses the same bounded borrowed parser as semantic authority.
pub(super) fn failure(
    data: &str,
    vendor: super::Vendor,
) -> Result<Option<super::TerminalEvent>, serde_json::Error> {
    super::super::error_fields::parse(data).map(|packet| packet.failure(vendor))
}
