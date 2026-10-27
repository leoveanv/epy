use super::SvgLimits;
use super::validation::svg_error;
use crate::Result;
use crate::cancellation::Cancellation;
use roxmltree::{Document, Node, NodeId};
use simplecss::{AttributeOperator, Element, PseudoClass, StyleSheet};
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub(super) struct StyleReferences<'a> {
    by_node: HashMap<NodeId, BTreeSet<&'a str>>,
}

impl<'a> StyleReferences<'a> {
    pub fn parse(
        document: &'a Document<'_>,
        limits: &SvgLimits,
        cancellation: &Cancellation,
    ) -> Result<Self> {
        let mut stylesheet = StyleSheet::new();
        let mut bytes = 0_usize;
        for node in document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "style")
        {
            let css = node.text().unwrap_or_default();
            bytes = bytes.saturating_add(css.len());
            if bytes > limits.css_bytes_max {
                return Err(svg_error("stylesheet size limit exceeded"));
            }
            stylesheet.parse_more(css);
        }
        if stylesheet.rules.len() > limits.css_rules_max
            || stylesheet
                .rules
                .iter()
                .any(|rule| rule.selector.to_string().len() > 1024)
        {
            return Err(svg_error("stylesheet complexity limit exceeded"));
        }
        let mut references = Self::default();
        let mut assignments = 0_usize;
        for rule in &stylesheet.rules {
            let urls = rule
                .declarations
                .iter()
                .flat_map(|declaration| url_references(declaration.value))
                .collect::<BTreeSet<_>>();
            if urls.is_empty() {
                continue;
            }
            for node in document.descendants().filter(|node| node.is_element()) {
                cancellation.check()?;
                if rule.selector.matches(&SvgElement(node)) {
                    assignments = assignments.saturating_add(urls.len());
                    if assignments > limits.nodes_max {
                        return Err(svg_error("stylesheet reference limit exceeded"));
                    }
                    references
                        .by_node
                        .entry(node.id())
                        .or_default()
                        .extend(urls.iter().copied());
                }
            }
        }
        Ok(references)
    }

    pub fn get(&self, id: NodeId) -> impl Iterator<Item = &'a str> {
        self.by_node
            .get(&id)
            .into_iter()
            .flat_map(|references| references.iter().copied())
    }
}

pub(super) fn url_references(value: &str) -> impl Iterator<Item = &str> {
    value
        .split("url(")
        .skip(1)
        .filter_map(|tail| {
            tail.split_once(')')
                .map(|(value, _)| value.trim().trim_matches(['\'', '"']))
        })
        .filter_map(|value| value.strip_prefix('#'))
}

struct SvgElement<'a, 'input>(Node<'a, 'input>);

impl Element for SvgElement<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(Self)
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        self.0
            .prev_siblings()
            .skip(1)
            .find(|node| node.is_element())
            .map(Self)
    }
    fn has_local_name(&self, name: &str) -> bool {
        self.0.tag_name().name() == name
    }
    fn attribute_matches(&self, local_name: &str, operator: AttributeOperator<'_>) -> bool {
        self.0
            .attribute(local_name)
            .is_some_and(|value| operator.matches(value))
    }
    fn pseudo_class_matches(&self, class: PseudoClass<'_>) -> bool {
        match class {
            PseudoClass::FirstChild => self.prev_sibling_element().is_none(),
            PseudoClass::Link => {
                self.0.tag_name().name() == "a" && self.0.attribute("href").is_some()
            }
            PseudoClass::Lang(language) => self
                .0
                .ancestors()
                .find_map(|node| node.attribute(("http://www.w3.org/XML/1998/namespace", "lang")))
                .is_some_and(|value| {
                    value == language
                        || value
                            .strip_prefix(language)
                            .is_some_and(|tail| tail.starts_with('-'))
                }),
            _ => false,
        }
    }
}
