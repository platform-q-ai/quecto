//! The board view's vocabulary (#2478): what the supervising session reads
//! of the boards its swarm environments host.

/// Why a swarm's board could not be answered (#2478).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwarmBoardRefusal {
    /// What was sent is not an environment ref (`C` and digits); it is
    /// never echoed.
    NotARef,
    /// No environment of this session's registry has that ref.
    UnknownSwarm(String),
    /// Another session's environment: the view shows this session's own,
    /// as `get_containers` lists them without `all`.
    OtherSession(String),
    /// Neither running nor retained: no board is read there.
    NotLive(String),
    /// The environment hosts no board this session can reach.
    NoBoard(String),
    /// The registry or the board could not be read.
    Unreadable(String),
}

impl std::fmt::Display for SwarmBoardRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotARef => write!(f, "swarm must be an environment ref such as C1"),
            Self::UnknownSwarm(swarm) => write!(f, "unknown swarm {swarm}"),
            Self::OtherSession(swarm) => write!(
                f,
                "swarm {swarm} is another session's; this view reads this session's own"
            ),
            Self::NotLive(swarm) => write!(
                f,
                "swarm {swarm} is neither running nor retained; its board is not read"
            ),
            Self::NoBoard(swarm) => write!(f, "swarm {swarm} hosts no board this session can read"),
            Self::Unreadable(error) => write!(f, "swarm board unreadable: {error}"),
        }
    }
}

/// A hosted board's revision, which every board write moves, and the id of
/// the run it counts (`None` before a run row exists): a board made again
/// starts its revision again under another run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedBoardRevision {
    pub run: Option<String>,
    pub revision: i64,
}

/// A board's projection as the board's adapter answers it (#2478): a JSON
/// object by construction, whose shape the adapter owns and the interface
/// presents; the capability passes it on unread.
#[derive(Debug, Clone, PartialEq)]
pub struct SwarmBoardSnapshot(serde_json::Value);

impl SwarmBoardSnapshot {
    /// `projection`, when it is a JSON object.
    pub fn object(projection: serde_json::Value) -> Option<Self> {
        match projection.is_object() {
            true => Some(Self(projection)),
            false => None,
        }
    }

    /// The projection, a JSON object.
    pub fn into_value(self) -> serde_json::Value {
        debug_assert!(self.0.is_object(), "a snapshot is an object");
        self.0
    }
}

/// What a poll of the boards found at one swarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardSighting {
    Board(HostedBoardRevision),
    /// The board could not be read this time (contention, a checkout that
    /// could not be told): what was last seen of it still stands.
    Unreadable(String),
}
