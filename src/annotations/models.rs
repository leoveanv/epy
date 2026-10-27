use crate::document::{NORMALIZATION_VERSION, SourceDocument};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationBook {
    pub id: String,
    pub fingerprint: String,
    pub title: String,
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TextAnchor {
    content_id: String,
    start: usize,
    end: usize,
    quote: String,
    prefix: String,
    suffix: String,
    normalization_version: u32,
}

#[derive(Deserialize)]
struct AnchorRecord {
    content_id: String,
    start: usize,
    end: usize,
    quote: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    suffix: String,
    #[serde(default = "normalization_version")]
    normalization_version: u32,
}

fn normalization_version() -> u32 {
    NORMALIZATION_VERSION
}

impl TextAnchor {
    pub fn new(
        content_id: impl Into<String>,
        start: usize,
        quote: impl Into<String>,
    ) -> Result<Self> {
        let quote = quote.into();
        let end = start.checked_add(quote.chars().count()).ok_or_else(|| {
            Error::Compatibility("Annotation range exceeds the document address space.".into())
        })?;
        Self::from_record(AnchorRecord {
            content_id: content_id.into(),
            start,
            end,
            quote,
            prefix: String::new(),
            suffix: String::new(),
            normalization_version: normalization_version(),
        })
    }

    fn from_record(record: AnchorRecord) -> Result<Self> {
        if record.end <= record.start || record.end - record.start != record.quote.chars().count() {
            return Err(Error::Compatibility(
                "An annotation must contain a nonempty text range.".into(),
            ));
        }
        Ok(Self {
            content_id: record.content_id,
            start: record.start,
            end: record.end,
            quote: record.quote,
            prefix: record.prefix,
            suffix: record.suffix,
            normalization_version: record.normalization_version,
        })
    }

    pub fn with_context(mut self, prefix: impl Into<String>, suffix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self.suffix = suffix.into();
        self
    }

    pub fn content_id(&self) -> &str {
        &self.content_id
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn quote(&self) -> &str {
        &self.quote
    }

    pub fn resolve(&self, document: &SourceDocument) -> Option<&Self> {
        if self.content_id != document.content_id
            || self.normalization_version != document.normalization_version
        {
            return None;
        }
        let actual = document
            .text
            .chars()
            .skip(self.start)
            .take(self.end - self.start);
        actual.eq(self.quote.chars()).then_some(self)
    }

    pub fn stable_location(&self, content_order: usize) -> String {
        format!("Location {}:{}", content_order + 1, self.start + 1)
    }
}

impl<'de> Deserialize<'de> for TextAnchor {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        Self::from_record(AnchorRecord::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationReference {
    #[serde(default)]
    pub headings: Vec<String>,
    pub location: String,
    #[serde(default)]
    pub page_start: Option<String>,
    #[serde(default)]
    pub page_end: Option<String>,
    #[serde(default = "normalization_version")]
    pub version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: String,
    pub book_id: String,
    pub anchor: TextAnchor,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub chapter: String,
    #[serde(default)]
    pub reference: Option<AnnotationReference>,
    #[serde(default)]
    pub content_order: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportLink {
    pub book_id: String,
    pub path: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub revision: i64,
    #[serde(default = "unexported_revision")]
    pub exported_revision: i64,
}

fn unexported_revision() -> i64 {
    -1
}
