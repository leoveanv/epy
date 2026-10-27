use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

pub struct DocumentInput {
    id: String,
    pub href: String,
    pub body: String,
}

impl DocumentInput {
    fn new(id: &str, href: &str, body: &str) -> Self {
        Self {
            id: id.into(),
            href: href.into(),
            body: body.into(),
        }
    }
}

pub struct AssetInput {
    pub path: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl AssetInput {
    #[allow(dead_code)]
    pub fn new(path: &str, media_type: &str, bytes: Vec<u8>) -> Self {
        Self {
            path: path.into(),
            media_type: media_type.into(),
            bytes,
        }
    }
}

pub struct EpubBuilder {
    version: String,
    pub documents: Vec<DocumentInput>,
    pub title: String,
    pub missing_resource: Option<String>,
    pub navigation: Option<String>,
    pub navigation_path: String,
    pub navigation_properties: String,
    pub use_ncx: bool,
    pub assets: Vec<AssetInput>,
}

impl EpubBuilder {
    pub fn new(version: &str) -> Self {
        Self {
            version: version.into(),
            title: "Synthetic & <title>".into(),
            missing_resource: None,
            navigation: None,
            navigation_path: "nav.xhtml".into(),
            navigation_properties: "nav".into(),
            use_ncx: version != "3.0",
            assets: Vec::new(),
            documents: vec![
                DocumentInput::new(
                    "first",
                    "first%20chapter.xhtml",
                    "<h1>First</h1><p>Café 👩‍🔬</p>",
                ),
                DocumentInput::new("second", "second.xhtml", "<p>Second</p>"),
                DocumentInput::new(
                    "notes",
                    "notes.xhtml",
                    "<p id='note'>Supplemental footnote</p>",
                ),
            ],
        }
    }

    pub fn build(&self, directory: &Path) -> PathBuf {
        let path = directory.join("synthetic.epub");
        let mut archive = zip::ZipWriter::new(File::create(&path).unwrap());
        let navigation = if !self.use_ncx {
            "<html xmlns='http://www.w3.org/1999/xhtml' xmlns:epub='http://www.idpf.org/2007/ops'><body><nav epub:type='toc'><ol><li><a href='second.xhtml#section'>Second <em>chapter</em></a></li><li><a href='first%20chapter.xhtml'>First</a></li></ol></nav></body></html>"
        } else {
            "<ncx xmlns='http://www.daisy.org/z3986/2005/ncx/'><navMap><navPoint><navLabel><text>Second chapter</text></navLabel><content src='second.xhtml#section'/></navPoint><navPoint><navLabel><text>First</text></navLabel><content src='first%20chapter.xhtml'/></navPoint></navMap></ncx>"
        };
        let navigation = self.navigation.as_deref().unwrap_or(navigation);
        let navigation =
            format!("<!DOCTYPE html SYSTEM 'https://example.invalid/unfetched.dtd'>{navigation}");
        let mut manifest = self
            .documents
            .iter()
            .map(|document| {
                format!(
                    "<item id='{}' href='{}' media-type='application/xhtml+xml'/>",
                    document.id, document.href
                )
            })
            .collect::<String>();
        for (index, asset) in self.assets.iter().enumerate() {
            manifest.push_str(&format!(
                "<item id='asset{index}' href='{}' media-type='{}'/>",
                asset.path, asset.media_type
            ));
        }
        manifest.push_str(&if !self.use_ncx {
            format!(
                "<item id='nav' href='{}' media-type='application/xhtml+xml' properties='{}'/>",
                self.navigation_path, self.navigation_properties
            )
        } else {
            format!(
                "<item id='ncx' href='{}' media-type='application/x-dtbncx+xml'/>",
                self.navigation_path
            )
        });
        let package = format!(
            "<package xmlns='http://www.idpf.org/2007/opf' version='{}'><metadata xmlns:dc='http://purl.org/dc/elements/1.1/'><dc:title>{}</dc:title><dc:creator>Fixture author</dc:creator></metadata><manifest>{manifest}</manifest><spine><itemref idref='second'/><itemref idref='first'/></spine></package>",
            self.version,
            html_escape::encode_text(&self.title)
        );
        for (name, text) in [
            ("mimetype", "application/epub+zip"),
            (
                "META-INF/container.xml",
                "<container xmlns='urn:oasis:names:tc:opendocument:xmlns:container'><rootfiles><rootfile full-path='OPS/package.opf'/></rootfiles></container>",
            ),
            ("OPS/package.opf", package.as_str()),
            (
                &format!("OPS/{}", self.navigation_path),
                navigation.as_str(),
            ),
            ("OPS/picture.png", "synthetic image bytes"),
        ] {
            archive
                .start_file(
                    name,
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            archive.write_all(text.as_bytes()).unwrap();
        }
        for asset in &self.assets {
            archive
                .start_file(format!("OPS/{}", asset.path), SimpleFileOptions::default())
                .unwrap();
            archive.write_all(&asset.bytes).unwrap();
        }
        for document in &self.documents {
            if self.missing_resource.as_deref() == Some(&document.id) {
                continue;
            }
            let name = format!(
                "OPS/{}",
                percent_encoding::percent_decode_str(&document.href).decode_utf8_lossy()
            );
            archive
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(document.body.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
        path
    }
}
