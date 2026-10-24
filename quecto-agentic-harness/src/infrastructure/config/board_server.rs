//! The board server's settings (epic #2705 B5): `board_server.listen`, the
//! TCP addresses `quecto board-server serve` listens on beside its Unix
//! socket (each on the allowlist, judged at start). Global-only.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoardServerConfig {
    /// `<ip>:<port>` each, e.g. the host's Tailscale address.
    #[serde(default)]
    pub listen: Vec<String>,
}
