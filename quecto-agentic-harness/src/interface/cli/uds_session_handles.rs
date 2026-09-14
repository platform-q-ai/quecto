//! What a dispatch loop needs from the sessions capability (#1970), as plain
//! handles.
//!
//! The interface declares the runtime inputs one loop hands over and the
//! store and controller handles it holds back; composition owns the concrete
//! graph between them (`composition::sessions`) and hands its builder in
//! through [`crate::interface::cli::CliContext`] as a
//! [`crate::interface::cli::SessionHandlesBuilder`], so no interface module
//! ever names the composition layer, constructs a store, or forms a path.
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::sessions::ports::SessionStore;
use crate::interface::uds::sessions::controller::ListSessionsController;

/// The runtime inputs of one loop the session handles are composed over.
pub struct SessionLoopInputs {
    /// The harness base directory the file store lives under.
    pub base_dir: PathBuf,
    /// A store the loop already holds (unit rigs, BDD fixtures); `None`
    /// composes the file store of `base_dir`.
    pub store: Option<Arc<dyn SessionStore>>,
}

/// The handles one loop holds on the sessions capability.
pub struct SessionHandles {
    /// The session store every session transaction of the loop runs
    /// against.
    pub store: Arc<dyn SessionStore>,
    /// List saved sessions (#1861): the UDS `list_sessions` command.
    pub list_sessions: Arc<ListSessionsController>,
}
