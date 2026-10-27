mod entities;
mod tokenizer;

pub(crate) use entities::unescape_html;
pub(crate) use tokenizer::{HtmlTag, HtmlToken, HtmlTokenizer};
