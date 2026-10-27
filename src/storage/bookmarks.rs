use super::Store;
use crate::Result;
use crate::reading::{Bookmark, ReadingState};
use rusqlite::{TransactionBehavior, params};
use sha1::{Digest, Sha1};

impl Store {
    pub fn insert_bookmark(
        &mut self,
        path: &str,
        name: &str,
        position: &ReadingState,
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO reading_states (filepath, content_id, textwidth, offset, screen_row) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(filepath) DO NOTHING",
            params![path, position.content_id, position.textwidth, position.offset, position.screen_row],
        )?;
        transaction.execute(
            "INSERT INTO bookmarks (id, filepath, name, content_id, textwidth, offset, screen_row) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![bookmark_id(path, name), path, name, position.content_id, position.textwidth, position.offset, position.screen_row],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn bookmarks(&self, path: &str) -> Result<Vec<Bookmark>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, content_id, textwidth, offset, screen_row FROM bookmarks WHERE filepath=? ORDER BY rowid",
        )?;
        Ok(statement
            .query_map([path], |row| {
                Ok(Bookmark {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    position: ReadingState {
                        content_id: row.get(2)?,
                        textwidth: row.get(3)?,
                        offset: row.get(4)?,
                        screen_row: row.get(5)?,
                    },
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_bookmark(&mut self, path: &str, name: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM bookmarks WHERE filepath=? AND name=?",
            params![path, name],
        )?;
        Ok(())
    }
}

pub fn bookmark_id(path: &str, name: &str) -> String {
    let mut hash = Sha1::new();
    hash.update(path.as_bytes());
    hash.update(name.as_bytes());
    format!("{:x}", hash.finalize())[..10].to_owned()
}
