use crate::Result;
use roxmltree::Document;

// #NOTE: EPUB navigation commonly declares a DTD. Accept declarations without resolving external entities or fetching resources.
pub(crate) fn parse_xml(xml: &str) -> Result<Document<'_>> {
    Ok(Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: 1_000_000,
            entity_resolver: None,
        },
    )?)
}
