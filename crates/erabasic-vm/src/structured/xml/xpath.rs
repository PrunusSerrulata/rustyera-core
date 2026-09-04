//! eraFL's portable, namespace-free `XPath` 1.0 subset.

use crate::ExecutionFailure;
use crate::structured::argument_failure;
use std::collections::HashMap;

use super::{XmlChild, XmlDocument, XmlSelection};
use parser::parse_xpath;

mod parser;
mod value;

use value::{
    xpath_bool_comparison, xpath_context_number, xpath_name_matches, xpath_number_comparison,
    xpath_parse_number, xpath_string_comparison,
};

impl XmlDocument {
    pub(crate) fn select(&self, path: &str) -> Result<Vec<XmlSelection>, ExecutionFailure> {
        let path = path.trim();
        if path.is_empty() || path == "." {
            return Ok(vec![XmlSelection {
                element_path: Vec::new(),
                attribute: None,
            }]);
        }
        let expression = parse_xpath(path)?;
        XPathEvaluator::new(self)?.select(&expression)
    }
}

struct XPathEvaluator<'a> {
    document: &'a XmlDocument,
    document_ranks: HashMap<XPathNode, usize>,
}

impl<'a> XPathEvaluator<'a> {
    fn new(document: &'a XmlDocument) -> Result<Self, ExecutionFailure> {
        let mut order = vec![XPathNode::Document];
        append_xpath_document_order(document, &[], &mut order)?;
        let document_ranks = order
            .into_iter()
            .enumerate()
            .map(|(rank, node)| (node, rank))
            .collect();
        Ok(Self {
            document,
            document_ranks,
        })
    }

    fn select(&self, expression: &XPathExpression) -> Result<Vec<XmlSelection>, ExecutionFailure> {
        let mut nodes = Vec::new();
        for location in &expression.paths {
            nodes.extend(self.evaluate_path(location, &XPathNode::Document)?);
        }
        self.sort_and_deduplicate_nodes(&mut nodes);
        nodes
            .into_iter()
            .map(|node| match node {
                XPathNode::Element(element_path) => Ok(XmlSelection {
                    element_path,
                    attribute: None,
                }),
                XPathNode::Attribute(element_path, attribute) => Ok(XmlSelection {
                    element_path,
                    attribute: Some(attribute),
                }),
                XPathNode::Document | XPathNode::Text(_, _) => Err(argument_failure(
                    "native.xpath.unsupported: XML_GET cannot return document or text nodes",
                )),
            })
            .collect()
    }

    fn evaluate_path(
        &self,
        path: &XPathPath,
        context: &XPathNode,
    ) -> Result<Vec<XPathNode>, ExecutionFailure> {
        let mut current = if path.absolute || matches!(context, &XPathNode::Document) {
            vec![XPathNode::Document]
        } else {
            vec![context.clone()]
        };
        for step in &path.steps {
            let mut next = Vec::new();
            for parent in &current {
                let mut candidates = self.xpath_step_candidates(parent, step)?;
                for predicate in &step.predicates {
                    let size = candidates.len();
                    let mut filtered = Vec::new();
                    for (index, candidate) in candidates.iter().enumerate() {
                        let context = XPathContext {
                            node: candidate,
                            position: index + 1,
                            size,
                        };
                        let value = self.evaluate_predicate(predicate, context)?;
                        let matches = match value {
                            XPathValue::Number(number) => xpath_number_comparison(
                                XPathComparison::Equal,
                                number,
                                xpath_context_number(context.position),
                            ),
                            value => Self::xpath_boolean(&value),
                        };
                        if matches {
                            filtered.push(candidate.clone());
                        }
                    }
                    candidates = filtered;
                }
                next.extend(candidates);
            }
            self.sort_and_deduplicate_nodes(&mut next);
            current = next;
        }
        Ok(current)
    }

    fn xpath_step_candidates(
        &self,
        parent: &XPathNode,
        step: &XPathStep,
    ) -> Result<Vec<XPathNode>, ExecutionFailure> {
        let mut output = Vec::new();
        match step.axis {
            XPathAxis::Child => self.xpath_child_nodes(parent, &step.test, &mut output)?,
            XPathAxis::Descendant => {
                self.xpath_descendant_nodes(parent, &step.test, &mut output)?;
            }
            XPathAxis::DescendantOrSelfAttribute => {
                let XPathTest::Attribute(name) = &step.test else {
                    return Err(
                        "native.xpath.unsupported: invalid descendant attribute step".into(),
                    );
                };
                match parent {
                    XPathNode::Document => {
                        self.xpath_attributes_for_element(&[], name, &mut output)?;
                        let mut paths = Vec::new();
                        self.descendant_paths(&[], "*", &mut paths)?;
                        for path in paths {
                            self.xpath_attributes_for_element(&path, name, &mut output)?;
                        }
                    }
                    XPathNode::Element(path) => {
                        self.xpath_attributes_for_element(path, name, &mut output)?;
                        let mut paths = Vec::new();
                        self.descendant_paths(path, "*", &mut paths)?;
                        for path in paths {
                            self.xpath_attributes_for_element(&path, name, &mut output)?;
                        }
                    }
                    XPathNode::Attribute(_, _) | XPathNode::Text(_, _) => {}
                }
            }
            XPathAxis::Attribute => {
                let XPathTest::Attribute(name) = &step.test else {
                    return Err("native.xpath.unsupported: invalid attribute step".into());
                };
                if let XPathNode::Element(path) = parent {
                    self.xpath_attributes_for_element(path, name, &mut output)?;
                }
            }
        }
        Ok(output)
    }

    fn xpath_child_nodes(
        &self,
        parent: &XPathNode,
        test: &XPathTest,
        output: &mut Vec<XPathNode>,
    ) -> Result<(), ExecutionFailure> {
        match parent {
            XPathNode::Document => {
                if let XPathTest::Element(name) = test
                    && xpath_name_matches(name, &self.document.root.name)
                {
                    output.push(XPathNode::Element(Vec::new()));
                }
            }
            XPathNode::Element(path) => {
                let element = self.element(path)?;
                for (index, child) in element.children.iter().enumerate() {
                    match (test, child) {
                        (XPathTest::Element(name), XmlChild::Element(child))
                            if xpath_name_matches(name, &child.name) =>
                        {
                            let mut child_path = path.clone();
                            child_path.push(index);
                            output.push(XPathNode::Element(child_path));
                        }
                        (XPathTest::Text, XmlChild::Text(_)) => {
                            output.push(XPathNode::Text(path.clone(), index));
                        }
                        _ => {}
                    }
                }
            }
            XPathNode::Attribute(_, _) | XPathNode::Text(_, _) => {}
        }
        Ok(())
    }

    fn xpath_descendant_nodes(
        &self,
        parent: &XPathNode,
        test: &XPathTest,
        output: &mut Vec<XPathNode>,
    ) -> Result<(), ExecutionFailure> {
        match parent {
            XPathNode::Document => {
                self.xpath_descendant_or_self_element(&[], test, output)?;
            }
            XPathNode::Element(path) => {
                let element = self.element(path)?;
                for (index, child) in element.children.iter().enumerate() {
                    if matches!(child, XmlChild::Element(_)) {
                        let mut child_path = path.clone();
                        child_path.push(index);
                        self.xpath_descendant_or_self_element(&child_path, test, output)?;
                    }
                }
            }
            XPathNode::Attribute(_, _) | XPathNode::Text(_, _) => {}
        }
        Ok(())
    }

    fn xpath_descendant_or_self_element(
        &self,
        path: &[usize],
        test: &XPathTest,
        output: &mut Vec<XPathNode>,
    ) -> Result<(), ExecutionFailure> {
        let element = self.element(path)?;
        if let XPathTest::Element(name) = test
            && xpath_name_matches(name, &element.name)
        {
            output.push(XPathNode::Element(path.to_vec()));
        }
        for (index, child) in element.children.iter().enumerate() {
            match (test, child) {
                (XPathTest::Text, XmlChild::Text(_)) => {
                    output.push(XPathNode::Text(path.to_vec(), index));
                }
                (_, XmlChild::Element(_)) => {
                    let mut child_path = path.to_vec();
                    child_path.push(index);
                    self.xpath_descendant_or_self_element(&child_path, test, output)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn xpath_attributes_for_element(
        &self,
        path: &[usize],
        name: &str,
        output: &mut Vec<XPathNode>,
    ) -> Result<(), ExecutionFailure> {
        let element = self.element(path)?;
        for (index, (candidate, _)) in element.attributes.iter().enumerate() {
            if xpath_name_matches(name, candidate) {
                output.push(XPathNode::Attribute(path.to_vec(), index));
            }
        }
        Ok(())
    }

    fn evaluate_predicate(
        &self,
        predicate: &XPathPredicate,
        context: XPathContext<'_>,
    ) -> Result<XPathValue, ExecutionFailure> {
        match predicate {
            XPathPredicate::Or(parts) => {
                for part in parts {
                    let value = self.evaluate_predicate(part, context)?;
                    if Self::xpath_boolean(&value) {
                        return Ok(XPathValue::Boolean(true));
                    }
                }
                Ok(XPathValue::Boolean(false))
            }
            XPathPredicate::And(parts) => {
                for part in parts {
                    let value = self.evaluate_predicate(part, context)?;
                    if !Self::xpath_boolean(&value) {
                        return Ok(XPathValue::Boolean(false));
                    }
                }
                Ok(XPathValue::Boolean(true))
            }
            XPathPredicate::Not(inner) => {
                let value = self.evaluate_predicate(inner, context)?;
                Ok(XPathValue::Boolean(!Self::xpath_boolean(&value)))
            }
            XPathPredicate::Compare(comparison, left, right) => {
                let left = self.evaluate_predicate(left, context)?;
                let right = self.evaluate_predicate(right, context)?;
                Ok(XPathValue::Boolean(self.xpath_compare(
                    *comparison,
                    &left,
                    &right,
                )?))
            }
            XPathPredicate::Contains(value, fragment) => {
                let value = self.evaluate_predicate(value, context)?;
                let fragment = self.evaluate_predicate(fragment, context)?;
                Ok(XPathValue::Boolean(
                    self.xpath_string(&value)?
                        .contains(&self.xpath_string(&fragment)?),
                ))
            }
            XPathPredicate::Concat(parts) => {
                let mut output = String::new();
                for part in parts {
                    let value = self.evaluate_predicate(part, context)?;
                    output.push_str(&self.xpath_string(&value)?);
                }
                Ok(XPathValue::String(output))
            }
            XPathPredicate::Union(paths) => {
                let mut nodes = Vec::new();
                for path in paths {
                    nodes.extend(self.evaluate_path(path, context.node)?);
                }
                self.sort_and_deduplicate_nodes(&mut nodes);
                Ok(XPathValue::Nodes(nodes))
            }
            XPathPredicate::Path(path) => {
                Ok(XPathValue::Nodes(self.evaluate_path(path, context.node)?))
            }
            XPathPredicate::String(value) => Ok(XPathValue::String(value.clone())),
            XPathPredicate::Number(value) => Ok(XPathValue::Number(*value)),
            XPathPredicate::Last => Ok(XPathValue::Number(xpath_context_number(context.size))),
        }
    }

    fn xpath_compare(
        &self,
        comparison: XPathComparison,
        left: &XPathValue,
        right: &XPathValue,
    ) -> Result<bool, ExecutionFailure> {
        if let XPathValue::Nodes(left_nodes) = left {
            if let XPathValue::Nodes(right_nodes) = right {
                for left in left_nodes {
                    let left = self.xpath_node_string(left)?;
                    for right in right_nodes {
                        let right = self.xpath_node_string(right)?;
                        let matches = if comparison.is_relational() {
                            xpath_number_comparison(
                                comparison,
                                xpath_parse_number(&left),
                                xpath_parse_number(&right),
                            )
                        } else {
                            xpath_string_comparison(comparison, &left, &right)
                        };
                        if matches {
                            return Ok(true);
                        }
                    }
                }
                return Ok(false);
            }
            if matches!(right, XPathValue::Boolean(_)) && !comparison.is_relational() {
                return Ok(xpath_bool_comparison(
                    comparison,
                    !left_nodes.is_empty(),
                    Self::xpath_boolean(right),
                ));
            }
            return left_nodes.iter().try_fold(false, |matched, node| {
                let node = self.xpath_node_string(node)?;
                let matches =
                    if comparison.is_relational() || matches!(right, XPathValue::Number(_)) {
                        xpath_number_comparison(
                            comparison,
                            xpath_parse_number(&node),
                            self.xpath_number(right)?,
                        )
                    } else {
                        xpath_string_comparison(comparison, &node, &self.xpath_string(right)?)
                    };
                Ok(matched || matches)
            });
        }
        if let XPathValue::Nodes(right_nodes) = right {
            if matches!(left, XPathValue::Boolean(_)) && !comparison.is_relational() {
                return Ok(xpath_bool_comparison(
                    comparison,
                    Self::xpath_boolean(left),
                    !right_nodes.is_empty(),
                ));
            }
            return right_nodes.iter().try_fold(false, |matched, node| {
                let node = self.xpath_node_string(node)?;
                let matches = if comparison.is_relational() || matches!(left, XPathValue::Number(_))
                {
                    xpath_number_comparison(
                        comparison,
                        self.xpath_number(left)?,
                        xpath_parse_number(&node),
                    )
                } else {
                    xpath_string_comparison(comparison, &self.xpath_string(left)?, &node)
                };
                Ok(matched || matches)
            });
        }
        if comparison.is_relational() {
            return Ok(xpath_number_comparison(
                comparison,
                self.xpath_number(left)?,
                self.xpath_number(right)?,
            ));
        }
        if matches!(left, XPathValue::Boolean(_)) || matches!(right, XPathValue::Boolean(_)) {
            return Ok(xpath_bool_comparison(
                comparison,
                Self::xpath_boolean(left),
                Self::xpath_boolean(right),
            ));
        }
        if matches!(left, XPathValue::Number(_)) || matches!(right, XPathValue::Number(_)) {
            return Ok(xpath_number_comparison(
                comparison,
                self.xpath_number(left)?,
                self.xpath_number(right)?,
            ));
        }
        Ok(xpath_string_comparison(
            comparison,
            &self.xpath_string(left)?,
            &self.xpath_string(right)?,
        ))
    }

    fn xpath_boolean(value: &XPathValue) -> bool {
        match value {
            XPathValue::Nodes(nodes) => !nodes.is_empty(),
            XPathValue::String(value) => !value.is_empty(),
            XPathValue::Number(value) => *value != 0.0 && !value.is_nan(),
            XPathValue::Boolean(value) => *value,
        }
    }

    fn xpath_string(&self, value: &XPathValue) -> Result<String, ExecutionFailure> {
        match value {
            XPathValue::Nodes(nodes) => nodes
                .first()
                .map(|node| self.xpath_node_string(node))
                .transpose()
                .map(Option::unwrap_or_default),
            XPathValue::String(value) => Ok(value.clone()),
            XPathValue::Number(value) => Ok(if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                value.to_string()
            }),
            XPathValue::Boolean(value) => Ok(if *value { "true" } else { "false" }.into()),
        }
    }

    fn xpath_number(&self, value: &XPathValue) -> Result<f64, ExecutionFailure> {
        Ok(match value {
            XPathValue::Nodes(_) | XPathValue::String(_) => {
                xpath_parse_number(&self.xpath_string(value)?)
            }
            XPathValue::Number(value) => *value,
            XPathValue::Boolean(value) => f64::from(u8::from(*value)),
        })
    }

    fn xpath_node_string(&self, node: &XPathNode) -> Result<String, ExecutionFailure> {
        match node {
            XPathNode::Document => Ok(self.document.root.inner_text()),
            XPathNode::Element(path) => Ok(self.element(path)?.inner_text()),
            XPathNode::Attribute(path, index) => self
                .element(path)?
                .attributes
                .get(*index)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| "XML selection path became invalid".into()),
            XPathNode::Text(path, index) => match self.element(path)?.children.get(*index) {
                Some(XmlChild::Text(value)) => Ok(value.clone()),
                _ => Err("XML selection path became invalid".into()),
            },
        }
    }

    fn sort_and_deduplicate_nodes(&self, nodes: &mut Vec<XPathNode>) {
        nodes.sort_by_key(|node| self.document_ranks.get(node).copied().unwrap_or(usize::MAX));
        nodes.dedup();
    }

    fn element(&self, path: &[usize]) -> Result<&super::XmlElement, ExecutionFailure> {
        self.document.element(path)
    }

    fn descendant_paths(
        &self,
        start: &[usize],
        name: &str,
        output: &mut Vec<Vec<usize>>,
    ) -> Result<(), ExecutionFailure> {
        self.document.descendant_paths(start, name, output)
    }
}

fn append_xpath_document_order(
    document: &XmlDocument,
    path: &[usize],
    output: &mut Vec<XPathNode>,
) -> Result<(), ExecutionFailure> {
    output.push(XPathNode::Element(path.to_vec()));
    let element = document.element(path)?;
    for index in 0..element.attributes.len() {
        output.push(XPathNode::Attribute(path.to_vec(), index));
    }
    for (index, child) in element.children.iter().enumerate() {
        match child {
            XmlChild::Text(_) => output.push(XPathNode::Text(path.to_vec(), index)),
            XmlChild::Element(_) => {
                let mut child_path = path.to_vec();
                child_path.push(index);
                append_xpath_document_order(document, &child_path, output)?;
            }
        }
    }
    Ok(())
}

struct XPathExpression {
    paths: Vec<XPathPath>,
}

#[derive(Clone, Debug)]
struct XPathPath {
    absolute: bool,
    steps: Vec<XPathStep>,
}

#[derive(Clone, Debug)]
struct XPathStep {
    axis: XPathAxis,
    test: XPathTest,
    predicates: Vec<XPathPredicate>,
}

#[derive(Clone, Copy, Debug)]
enum XPathAxis {
    Child,
    Descendant,
    DescendantOrSelfAttribute,
    Attribute,
}

#[derive(Clone, Debug)]
enum XPathTest {
    Element(String),
    Attribute(String),
    Text,
}

#[derive(Clone, Debug)]
enum XPathPredicate {
    Or(Vec<Self>),
    And(Vec<Self>),
    Not(Box<Self>),
    Compare(XPathComparison, Box<Self>, Box<Self>),
    Contains(Box<Self>, Box<Self>),
    Concat(Vec<Self>),
    Union(Vec<XPathPath>),
    Path(XPathPath),
    String(String),
    Number(f64),
    Last,
}

#[derive(Clone, Copy, Debug)]
enum XPathComparison {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl XPathComparison {
    fn is_relational(self) -> bool {
        matches!(
            self,
            Self::Less | Self::LessOrEqual | Self::Greater | Self::GreaterOrEqual
        )
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum XPathNode {
    Document,
    Element(Vec<usize>),
    Attribute(Vec<usize>, usize),
    Text(Vec<usize>, usize),
}

#[derive(Clone, Debug)]
enum XPathValue {
    Nodes(Vec<XPathNode>),
    String(String),
    Number(f64),
    Boolean(bool),
}

#[derive(Clone, Copy)]
struct XPathContext<'a> {
    node: &'a XPathNode,
    position: usize,
    size: usize,
}
