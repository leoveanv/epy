use crate::{Error, Result};
use rusqlite::{Connection, TransactionBehavior};
use std::time::Duration;

pub(super) fn configure_connection(connection: &Connection) -> Result<()> {
    connection.busy_timeout(Duration::from_secs(10))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}

pub(super) fn schema_version(connection: &Connection) -> Result<i64> {
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

pub(super) fn check_version(connection: &Connection) -> Result<()> {
    if schema_version(connection)? != 1 {
        return Err(Error::Compatibility(
            "The ers database schema is not supported by this reader.".into(),
        ));
    }
    Ok(())
}

pub(super) fn initialize_schema(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(include_str!("schema.sql"))?;
    validate_schema(&transaction)?;
    transaction.pragma_update(None, "user_version", 1)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn validate_schema(connection: &Connection) -> Result<()> {
    for query in [
        "SELECT filepath, content_id, textwidth, offset, screen_row FROM reading_states LIMIT 0",
        "SELECT last_read, filepath, title, author, reading_progress FROM library LIMIT 0",
        "SELECT id, filepath, name, content_id, textwidth, offset, screen_row FROM bookmarks LIMIT 0",
        "SELECT id, fingerprint, title, author FROM annotation_books LIMIT 0",
        "SELECT path, book_id FROM annotation_locations LIMIT 0",
        "SELECT id, book_id, content_order, start, payload FROM annotations LIMIT 0",
        "SELECT book_id, path, digest, revision, exported_revision FROM annotation_exports LIMIT 0",
    ] {
        connection.prepare(query)?;
    }
    Ok(())
}
