use super::book::BookWorker;
use crate::Result;
use crate::app::requests::{LibraryCommand, LibraryReply};

impl BookWorker {
    pub(super) fn library(&mut self, command: LibraryCommand) -> Result<LibraryReply> {
        match command {
            LibraryCommand::List(departure) => {
                self.save(&departure)?;
                Ok(LibraryReply::Entries(self.store.history()?))
            }
            LibraryCommand::Remove(path) => {
                self.store.delete_history(&path)?;
                Ok(LibraryReply::Removed {
                    path,
                    entries: self.store.history()?,
                })
            }
            LibraryCommand::Switch { path, departure } => {
                self.save(&departure)?;
                Ok(LibraryReply::SwitchReady(path))
            }
        }
    }
}
