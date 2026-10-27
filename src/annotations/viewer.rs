use crate::document::text::wrap_text;

pub fn note_view_lines(quote: &str, note: &str, width: usize) -> Vec<String> {
    let mut lines = vec!["Quotation".into(), String::new()];
    for paragraph in quote.split('\n') {
        let wrapped = wrap_text(paragraph, width);
        if wrapped.is_empty() {
            lines.push(String::new());
        } else {
            lines.extend(wrapped);
        }
    }
    lines.extend([String::new(), "Note".into(), String::new()]);
    for paragraph in note.split('\n') {
        let wrapped = wrap_text(paragraph, width);
        if wrapped.is_empty() {
            lines.push(String::new());
        } else {
            lines.extend(wrapped);
        }
    }
    lines
}
