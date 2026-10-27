pub mod annotations;
pub mod book;
pub mod cancellation;
pub mod configuration;
pub mod document;
pub mod export;
pub mod images;
pub mod integrations;
pub mod reading;
pub mod search;
pub mod storage;

mod filesystem;
mod worker;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("The operation was canceled.")]
    Canceled,
    #[error("{0}")]
    Compatibility(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Archive(#[from] zip::result::ZipError),
    #[error(transparent)]
    Xml(#[from] roxmltree::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
