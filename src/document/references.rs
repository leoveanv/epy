#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceHeading {
    pub offset: usize,
    pub title: String,
    pub level: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferencePage {
    pub offset: usize,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentReferences {
    pub headings: Vec<ReferenceHeading>,
    pub pages: Vec<ReferencePage>,
}

pub fn clean_page_label(label: &str) -> Option<String> {
    let label = label
        .split(crate::document::text::python_whitespace)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let label = if label
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("page "))
    {
        &label[5..]
    } else {
        &label
    };
    (!label.is_empty()).then(|| label.into())
}
