use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub last_read: String,
    pub filepath: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub reading_progress: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookMetadata {
    pub title: Option<String>,
    pub creator: Option<String>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub language: Option<String>,
    pub format: Option<String>,
    pub identifier: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TocEntry {
    pub label: String,
    pub content_index: usize,
    pub section: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct ResourceLimits {
    pub resource_bytes_max: u64,
    pub archive_bytes_max: u64,
    pub archive_entries_max: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            resource_bytes_max: 32 * 1024 * 1024,
            archive_bytes_max: 512 * 1024 * 1024,
            archive_entries_max: 100_000,
        }
    }
}
