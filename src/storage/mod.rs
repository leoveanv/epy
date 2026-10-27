mod annotation_records;
mod bookmarks;
mod connection;
mod export_links;
mod reading;

pub use bookmarks::bookmark_id;

use crate::Result;
use connection::{
    check_version, configure_connection, initialize_schema, schema_version, validate_schema,
};
use rusqlite::Connection;
use std::path::Path;

use crate::annotations::{Annotation, ExportLink};
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSnapshot {
    pub link: ExportLink,
    pub annotations: Vec<Annotation>,
}

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let existed = path.exists();
        let mut connection = Connection::open(path)?;
        configure_connection(&connection)?;
        if existed {
            check_version(&connection)?;
        } else {
            initialize_schema(&mut connection)?;
        }
        validate_schema(&connection)?;
        Ok(Self { connection })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut connection = Connection::open_in_memory()?;
        configure_connection(&connection)?;
        initialize_schema(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> Result<i64> {
        schema_version(&self.connection)
    }
}
