use super::annotation_records::read_annotations;
use super::{ExportSnapshot, Store};
use crate::Result;
use crate::annotations::ExportLink;
use rusqlite::{Connection, OptionalExtension, params};

impl Store {
    pub fn export_link(&self, book_id: &str) -> Result<Option<ExportLink>> {
        read_link(&self.connection, book_id)
    }

    pub fn set_link(&mut self, link: &ExportLink) -> Result<()> {
        self.connection.execute(
            "INSERT INTO annotation_exports (book_id, path, digest, revision, exported_revision) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(book_id) DO UPDATE SET path=excluded.path, digest=excluded.digest,
             revision=excluded.revision, exported_revision=excluded.exported_revision",
            params![link.book_id, link.path, link.digest, link.revision, link.exported_revision],
        )?;
        Ok(())
    }

    pub fn unlink(&mut self, book_id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM annotation_exports WHERE book_id=?", [book_id])?;
        Ok(())
    }

    pub fn mark_exported(&mut self, link: &ExportLink, digest: &str) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE annotation_exports SET digest=?, exported_revision=? WHERE book_id=? AND path=? AND digest IS ? AND exported_revision=? AND revision>=?",
            params![digest, link.revision, link.book_id, link.path, link.digest, link.exported_revision, link.revision],
        )?;
        Ok(changed == 1)
    }

    pub fn export_snapshot(&mut self, book_id: &str) -> Result<Option<ExportSnapshot>> {
        let transaction = self.connection.transaction()?;
        let snapshot = read_link(&transaction, book_id)?
            .map(|link| {
                read_annotations(&transaction, book_id)
                    .map(|annotations| ExportSnapshot { link, annotations })
            })
            .transpose()?;
        transaction.commit()?;
        Ok(snapshot)
    }
}

fn read_link(connection: &Connection, book_id: &str) -> Result<Option<ExportLink>> {
    Ok(connection.query_row(
        "SELECT book_id, path, digest, revision, exported_revision FROM annotation_exports WHERE book_id=?", [book_id],
        |row| Ok(ExportLink { book_id: row.get(0)?, path: row.get(1)?, digest: row.get(2)?, revision: row.get(3)?, exported_revision: row.get(4)? }),
    ).optional()?)
}
