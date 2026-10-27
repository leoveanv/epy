use crate::book::LibraryEntry;
use crate::reading::SaveRequest;

#[derive(Clone, Debug)]
pub enum LibraryCommand {
    List(SaveRequest),
    Remove(String),
    Switch {
        path: String,
        departure: SaveRequest,
    },
}

impl LibraryCommand {
    pub fn departure(&self) -> Option<&SaveRequest> {
        match self {
            Self::List(departure) | Self::Switch { departure, .. } => Some(departure),
            Self::Remove(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LibraryJob {
    pub request_id: u64,
    pub command: LibraryCommand,
}

#[derive(Clone, Debug)]
pub enum LibraryReply {
    Entries(Vec<LibraryEntry>),
    Removed {
        path: String,
        entries: Vec<LibraryEntry>,
    },
    SwitchReady(String),
}

#[derive(Clone, Debug)]
pub struct LibraryResult {
    pub request_id: u64,
    pub reply: std::result::Result<LibraryReply, String>,
}
