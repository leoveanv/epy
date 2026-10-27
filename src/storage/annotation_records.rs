use super::Store;
use crate::annotations::{Annotation, AnnotationBook, ReassociationSource};
use crate::{Error, Result};
use rusqlite::{Connection, TransactionBehavior, params};

impl Store {
    pub fn identify_book(
        &mut self,
        proposed: &AnnotationBook,
        resolved_path: &str,
    ) -> Result<AnnotationBook> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO annotation_books (id, fingerprint, title, author) VALUES (?, ?, ?, ?) ON CONFLICT(fingerprint) DO NOTHING",
            params![proposed.id, proposed.fingerprint, proposed.title, proposed.author],
        )?;
        let book = transaction.query_row(
            "SELECT id, fingerprint, title, author FROM annotation_books WHERE fingerprint=?",
            [&proposed.fingerprint],
            book_from_row,
        )?;
        transaction.execute(
            "INSERT INTO annotation_locations (path, book_id) VALUES (?, ?) ON CONFLICT(path) DO UPDATE SET book_id=excluded.book_id",
            params![resolved_path, book.id],
        )?;
        transaction.commit()?;
        Ok(book)
    }

    pub fn books(&self) -> Result<Vec<AnnotationBook>> {
        let mut statement = self.connection.prepare(
            "SELECT id, fingerprint, title, author FROM annotation_books ORDER BY title, id",
        )?;
        Ok(statement
            .query_map([], book_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn annotations(&self, book_id: &str) -> Result<Vec<Annotation>> {
        read_annotations(&self.connection, book_id)
    }

    pub fn save_annotation(&mut self, annotation: &Annotation) -> Result<()> {
        let payload = serde_json::to_string(annotation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO annotations (id, book_id, content_order, start, payload) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET book_id=excluded.book_id, content_order=excluded.content_order,
             start=excluded.start, payload=excluded.payload",
            params![annotation.id, annotation.book_id, annotation.content_order, annotation.anchor.start(), payload],
        )?;
        transaction.execute(
            "UPDATE annotation_exports SET revision=revision+1 WHERE book_id=?",
            [&annotation.book_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn insert_annotations(&mut self, annotations: &[Annotation]) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for annotation in annotations {
            transaction.execute("INSERT INTO annotations (id, book_id, content_order, start, payload) VALUES (?, ?, ?, ?, ?)", params![annotation.id, annotation.book_id, annotation.content_order, annotation.anchor.start(), serde_json::to_string(annotation)?])?;
            transaction.execute(
                "UPDATE annotation_exports SET revision=revision+1 WHERE book_id=?",
                [&annotation.book_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn reassociation_sources(&self, target_book_id: &str) -> Result<Vec<ReassociationSource>> {
        let mut statement = self.connection.prepare("SELECT b.id, b.fingerprint, b.title, b.author, COUNT(a.id) FROM annotation_books b JOIN annotations a ON a.book_id=b.id WHERE b.id!=? GROUP BY b.id ORDER BY b.title, b.id")?;
        Ok(statement
            .query_map([target_book_id], |row| {
                Ok(ReassociationSource {
                    book: book_from_row(row)?,
                    annotation_count: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_annotation(&mut self, annotation: &Annotation) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM annotations WHERE id=? AND book_id=?",
            params![annotation.id, annotation.book_id],
        )?;
        transaction.execute(
            "UPDATE annotation_exports SET revision=revision+1 WHERE book_id=?",
            [&annotation.book_id],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn book_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AnnotationBook> {
    Ok(AnnotationBook {
        id: row.get(0)?,
        fingerprint: row.get(1)?,
        title: row.get(2)?,
        author: row.get(3)?,
    })
}

pub(super) fn read_annotations(connection: &Connection, book_id: &str) -> Result<Vec<Annotation>> {
    let mut statement = connection.prepare(
        "SELECT id, book_id, content_order, start, payload FROM annotations WHERE book_id=? ORDER BY content_order, start, id",
    )?;
    let mut rows = statement.query([book_id])?;
    let mut annotations = Vec::new();
    while let Some(row) = rows.next()? {
        let id = row.get::<_, String>(0)?;
        let annotation =
            serde_json::from_str::<Annotation>(&row.get::<_, String>(4)?).map_err(|error| {
                Error::Compatibility(format!(
                    "Annotation {id} contains an invalid payload: {error}"
                ))
            })?;
        if annotation.id != id
            || annotation.book_id != row.get::<_, String>(1)?
            || annotation.content_order != row.get::<_, usize>(2)?
            || annotation.anchor.start() != row.get::<_, usize>(3)?
        {
            return Err(Error::Compatibility(format!(
                "Annotation {id} payload disagrees with its stored identity or range."
            )));
        }
        annotations.push(annotation);
    }
    Ok(annotations)
}
