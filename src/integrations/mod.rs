mod browser;
mod dictionary;
pub use dictionary::{
    DICTIONARY_QUERY_BYTES_MAX, DictionaryCommand, DictionaryDefinition, DictionaryJob,
    DictionaryLimits, DictionaryOptions, DictionaryPage, DictionaryResult, DictionaryWorker,
    ExecutableSearch,
};
mod viewer;
pub use browser::{BrowserCommand, open_browser};
pub use viewer::{ImageViewer, viewer_command};
