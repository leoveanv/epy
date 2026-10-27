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
