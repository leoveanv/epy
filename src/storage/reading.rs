use super::Store;
use crate::Result;
use crate::book::LibraryEntry;
use crate::reading::ReadingState;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

impl Store {
    pub fn reading_state(&self, path: &str) -> Result<Option<ReadingState>> {
        Ok(self.connection.query_row(
            "SELECT content_id, textwidth, offset, screen_row FROM reading_states WHERE filepath=?",
            [path],
            reading_state_from_row,
        ).optional()?)
    }

    pub fn save_reading_state(&mut self, path: &str, position: &ReadingState) -> Result<()> {
        write_reading_position(&self.connection, path, position)
    }

    pub fn save_library_entry(&mut self, entry: &LibraryEntry) -> Result<()> {
        write_library_entry(&self.connection, entry)
    }

    pub fn save_session(&mut self, entry: &LibraryEntry, position: &ReadingState) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_reading_position(&transaction, &entry.filepath, position)?;
        write_library_entry(&transaction, entry)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn history(&self) -> Result<Vec<LibraryEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT last_read, filepath, title, author, reading_progress FROM library ORDER BY last_read DESC",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(LibraryEntry {
                    last_read: row.get(0)?,
                    filepath: row.get(1)?,
                    title: row.get(2)?,
                    author: row.get(3)?,
                    reading_progress: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_history(&mut self, path: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM reading_states WHERE filepath=?", [path])?;
        Ok(())
    }
}

fn write_reading_position(
    connection: &Connection,
    path: &str,
    position: &ReadingState,
) -> Result<()> {
    connection.execute(
        "INSERT INTO reading_states (filepath, content_id, textwidth, offset, screen_row)
             VALUES (?, ?, ?, ?, ?) ON CONFLICT(filepath) DO UPDATE SET
             content_id=excluded.content_id, textwidth=excluded.textwidth,
             offset=excluded.offset, screen_row=excluded.screen_row",
        params![
            path,
            position.content_id,
            position.textwidth,
            position.offset,
            position.screen_row
        ],
    )?;
    Ok(())
}

fn write_library_entry(connection: &Connection, entry: &LibraryEntry) -> Result<()> {
    connection.execute(
            "INSERT INTO library (last_read, filepath, title, author, reading_progress)
             VALUES (?, ?, ?, ?, ?) ON CONFLICT(filepath) DO UPDATE SET
             last_read=excluded.last_read, title=excluded.title,
             author=excluded.author, reading_progress=COALESCE(excluded.reading_progress, library.reading_progress)",
            params![
                entry.last_read,
                entry.filepath,
                entry.title,
                entry.author,
                entry.reading_progress
            ],
        )?;
    Ok(())
}

fn reading_state_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReadingState> {
    Ok(ReadingState {
        content_id: row.get(0)?,
        textwidth: row.get(1)?,
        offset: row.get(2)?,
        screen_row: row.get(3)?,
    })
}
