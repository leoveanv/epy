use std::path::{Path, PathBuf};

pub struct FictionBookBuilder {
    title: String,
    pub first_body: String,
    pub notes_body: String,
    binary: String,
}

impl FictionBookBuilder {
    pub fn new() -> Self {
        Self { title: "A café story".into(), first_body: "<section id='chapter-one'><title><p>Chapter one</p></title><p>Opening passage <emphasis>with emphasis</emphasis><a l:href='#note-one' type='note'>1</a>.</p><image l:href='#picture'/><section id='nested'><title><p>A nested scene</p></title><p>Nested passage.</p></section></section><section id='chapter-two'><title><p>Chapter two</p></title><p>Last passage.</p></section>".into(), notes_body: "<section id='note-one'><title><p>Note one</p></title><p>Supplemental note.</p></section>".into(), binary: "aGVsbG8=".into() }
    }

    pub fn markup(&self) -> String {
        format!(
            "<?xml version='1.0' encoding='UTF-8'?><FictionBook xmlns='http://www.gribuser.ru/xml/fictionbook/2.0' xmlns:l='http://www.w3.org/1999/xlink'><description><title-info><author><first-name>Fixture</first-name><middle-name>Q.</middle-name><last-name>Author</last-name></author><book-title>{}</book-title><lang>en</lang><date value='2026-01-01'>2026</date></title-info><document-info><id>synthetic-fb2-id</id></document-info></description><body>{}</body><body name='notes'>{}</body><binary id='picture' content-type='image/png'>{}</binary></FictionBook>",
            html_escape::encode_text(&self.title),
            self.first_body,
            self.notes_body,
            self.binary
        )
    }

    pub fn build(&self, root: &Path) -> PathBuf {
        let path = root.join("synthetic.fb2");
        std::fs::write(&path, self.markup()).unwrap();
        path
    }
}
