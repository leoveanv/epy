use crate::book::epub::join_resource;
use crate::book::xml::parse_xml;
use crate::document::clean_page_label;
use crate::{Error, Result};
use percent_encoding::percent_decode_str;
use roxmltree::Node;
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BookNavigation {
    pub sections: Vec<NavigationSection>,
    pub pages: Vec<NavigationPage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationSection {
    pub content_id: String,
    pub fragment: Option<String>,
    pub headings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationPage {
    pub content_id: String,
    pub fragment: Option<String>,
    pub label: String,
}

struct HeadingPath {
    title: String,
    parent: Option<Arc<HeadingPath>>,
}
struct SectionFrame<'a, 'input> {
    node: Node<'a, 'input>,
    parents: Option<Arc<HeadingPath>>,
    depth: usize,
}
struct Target {
    content_id: String,
    fragment: Option<String>,
}
#[derive(Clone, Copy)]
enum Syntax {
    Html,
    Ncx,
}

struct NavigationParser<'a> {
    base: &'a str,
    navigation: BookNavigation,
    bytes: usize,
}

fn local(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}
fn children<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Vec<Node<'a, 'input>> {
    node.children()
        .filter(|child| local(*child, name))
        .collect()
}
fn text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|child| child.text())
        .collect::<String>()
        .split(crate::document::text::python_whitespace)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn target(base: &str, href: &str) -> Result<Option<Target>> {
    let (path, fragment) = href
        .split_once('#')
        .map_or((href, None), |(path, fragment)| (path, Some(fragment)));
    if path.starts_with('/')
        || path
            .split('/')
            .next()
            .is_some_and(|part| part.contains(':'))
    {
        return Ok(None);
    }
    let path = percent_decode_str(path).decode_utf8_lossy();
    let content_id = if path.is_empty() {
        base.into()
    } else {
        join_resource(base, &path)?
    };
    let fragment = fragment
        .filter(|fragment| !fragment.is_empty())
        .map(|fragment| {
            percent_decode_str(fragment)
                .decode_utf8_lossy()
                .into_owned()
        });
    Ok(Some(Target {
        content_id,
        fragment,
    }))
}

impl NavigationParser<'_> {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > 32 * 1024 * 1024
            || self.navigation.sections.len() + self.navigation.pages.len() >= 100_000
        {
            return Err(Error::Compatibility(
                "The ebook navigation exceeds its metadata limits.".into(),
            ));
        }
        Ok(())
    }

    fn sections(&mut self, root: Node<'_, '_>, syntax: Syntax) -> Result<()> {
        let name = match syntax {
            Syntax::Html => "li",
            Syntax::Ncx => "navPoint",
        };
        let mut pending = children(root, name)
            .into_iter()
            .rev()
            .map(|node| SectionFrame {
                node,
                parents: None,
                depth: 1,
            })
            .collect::<Vec<_>>();
        while let Some(frame) = pending.pop() {
            if frame.depth > 64 {
                return Err(Error::Compatibility(
                    "The ebook navigation exceeds its nesting limit.".into(),
                ));
            }
            let label = match syntax {
                Syntax::Html => children(frame.node, "a")
                    .into_iter()
                    .next()
                    .or_else(|| children(frame.node, "span").into_iter().next()),
                Syntax::Ncx => children(frame.node, "navLabel").into_iter().next(),
            };
            let title = label.map(text).unwrap_or_default();
            let path = if title.is_empty() {
                frame.parents
            } else {
                Some(Arc::new(HeadingPath {
                    title,
                    parent: frame.parents,
                }))
            };
            let href = match syntax {
                Syntax::Html => label.and_then(|node| node.attribute("href")),
                Syntax::Ncx => children(frame.node, "content")
                    .into_iter()
                    .next()
                    .and_then(|node| node.attribute("src")),
            };
            if let Some(href) = href
                && let Some(target) = target(self.base, href)?
            {
                let mut headings = Vec::new();
                let mut ancestor = path.as_ref();
                let mut bytes =
                    target.content_id.len() + target.fragment.as_ref().map_or(0, String::len);
                while let Some(heading) = ancestor {
                    bytes = bytes.saturating_add(heading.title.len());
                    headings.push(heading.title.as_str());
                    ancestor = heading.parent.as_ref();
                }
                self.charge(bytes)?;
                headings.reverse();
                self.navigation.sections.push(NavigationSection {
                    content_id: target.content_id,
                    fragment: target.fragment,
                    headings: headings.into_iter().map(str::to_owned).collect(),
                });
            }
            let nested = match syntax {
                Syntax::Html => children(frame.node, "ol")
                    .into_iter()
                    .flat_map(|list| children(list, "li"))
                    .collect::<Vec<_>>(),
                Syntax::Ncx => children(frame.node, "navPoint"),
            };
            pending.extend(nested.into_iter().rev().map(|node| SectionFrame {
                node,
                parents: path.clone(),
                depth: frame.depth + 1,
            }));
        }
        Ok(())
    }

    fn page(&mut self, href: &str, label: &str) -> Result<()> {
        if let Some(label) = clean_page_label(label)
            && let Some(target) = target(self.base, href)?
        {
            self.charge(
                target.content_id.len()
                    + target.fragment.as_ref().map_or(0, String::len)
                    + label.len(),
            )?;
            self.navigation.pages.push(NavigationPage {
                content_id: target.content_id,
                fragment: target.fragment,
                label,
            });
        }
        Ok(())
    }
}

pub fn parse_navigation(markup: &str, content_id: &str) -> Result<BookNavigation> {
    let document = parse_xml(markup)?;
    let mut parser = NavigationParser {
        base: content_id,
        navigation: BookNavigation::default(),
        bytes: 0,
    };
    for node in document.descendants().filter(Node::is_element) {
        match node.tag_name().name() {
            "navMap" => parser.sections(node, Syntax::Ncx)?,
            "pageTarget" => {
                let label = children(node, "navLabel")
                    .into_iter()
                    .next()
                    .map(text)
                    .unwrap_or_else(|| node.attribute("value").unwrap_or_default().into());
                if let Some(href) = children(node, "content")
                    .into_iter()
                    .next()
                    .and_then(|node| node.attribute("src"))
                {
                    parser.page(href, &label)?;
                }
            }
            "nav" => {
                let kinds = node
                    .attribute(("http://www.idpf.org/2007/ops", "type"))
                    .unwrap_or_default()
                    .split_whitespace()
                    .collect::<Vec<_>>();
                if kinds.contains(&"toc") {
                    for list in children(node, "ol") {
                        parser.sections(list, Syntax::Html)?;
                    }
                } else if kinds.contains(&"page-list") {
                    for link in node.descendants().filter(|node| local(*node, "a")) {
                        if let Some(href) = link.attribute("href") {
                            let label = link
                                .attribute("title")
                                .map(str::to_owned)
                                .unwrap_or_else(|| text(link));
                            parser.page(href, &label)?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(parser.navigation)
}
