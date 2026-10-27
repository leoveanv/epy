pub(crate) mod html;
pub mod layout;
pub mod links;
mod references;
mod rendered;
mod source;
pub mod text;

pub use references::{DocumentReferences, ReferenceHeading, ReferencePage, clean_page_label};
pub use rendered::RenderedDocument;
pub(crate) use source::NORMALIZATION_VERSION;
pub use source::SourceDocument;
