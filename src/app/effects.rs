use std::sync::Arc;

use crate::annotations::{AnnotationJob, LinkJob, ReassociationJob};
use crate::reading::{BookmarkRequest, LoadRequest, SaveRequest};

pub enum ReaderEffect {
    Load(LoadRequest),
    Window(crate::reading::WindowRequest),
    Hyperlink(crate::reading::HyperlinkRequest),
    Browser(url::Url),
    Image(crate::images::ImageJob),
    CancelImage,
    Viewer(Arc<crate::images::ImagePayload>),
    Quit(SaveRequest),
    Bookmark(BookmarkRequest),
    Annotation(AnnotationJob),
    Link(LinkJob),
    Reassociation(ReassociationJob),
    Search(crate::search::SearchJob),
    CancelSearch,
    Dictionary(crate::integrations::DictionaryJob),
    CancelDictionary,
    Library(crate::app::requests::LibraryJob),
    OpenBook(String),
    CancelBookOpen,
    ForgetBookSession(String),
}
