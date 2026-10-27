pub(crate) const NORMALIZATION_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDocument {
    pub content_id: String,
    pub text: String,
    pub normalization_version: u32,
}

impl SourceDocument {
    pub fn new(content_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            content_id: content_id.into(),
            text: text.into(),
            normalization_version: NORMALIZATION_VERSION,
        }
    }
}
