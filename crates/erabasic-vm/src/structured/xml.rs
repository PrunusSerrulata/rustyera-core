//! Portable XML tree, mutation, and the supported deterministic `XPath` subset.

use super::{
    Event, NativeCallRequest, Reader, XmlChild, XmlDocument, XmlElement, XmlMutation, XmlSelection,
    optional_integer, optional_string, resolve_predefined_entity, string_argument,
};

#[allow(clippy::too_many_lines)]
pub(super) fn parse_xml(input: &str) -> Result<XmlDocument, String> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut stack = Vec::<XmlElement>::new();
    let mut root = None;
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(start) => {
                let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                let attributes = start
                    .attributes()
                    .map(|attribute| {
                        let attribute = attribute.map_err(|error| error.to_string())?;
                        Ok((
                            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                            attribute
                                .decode_and_unescape_value(reader.decoder())
                                .map_err(|error| error.to_string())?
                                .into_owned(),
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                stack.push(XmlElement {
                    name,
                    attributes,
                    children: Vec::new(),
                });
            }
            Event::Empty(start) => {
                let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                let element = XmlElement {
                    name,
                    attributes: start
                        .attributes()
                        .map(|attribute| {
                            let attribute = attribute.map_err(|error| error.to_string())?;
                            Ok((
                                String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                                attribute
                                    .decode_and_unescape_value(reader.decoder())
                                    .map_err(|error| error.to_string())?
                                    .into_owned(),
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                    children: Vec::new(),
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(XmlChild::Element(element));
                } else if root.replace(element).is_some() {
                    return Err("XML contains more than one root element".into());
                }
            }
            Event::Text(text) => {
                let value = text.decode().map_err(|error| error.to_string())?;
                let value = quick_xml::escape::unescape(&value)
                    .map_err(|error| error.to_string())?
                    .into_owned();
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(XmlChild::Text(value));
                } else if !value.trim().is_empty() {
                    return Err("XML text appears outside the root element".into());
                }
            }
            Event::CData(text) => {
                let value = text
                    .decode()
                    .map_err(|error| error.to_string())?
                    .into_owned();
                let parent = stack
                    .last_mut()
                    .ok_or("XML CDATA appears outside the root element")?;
                parent.children.push(XmlChild::Text(value));
            }
            Event::GeneralRef(reference) => {
                let reference = reference.decode().map_err(|error| error.to_string())?;
                let value = if let Some(number) = reference.strip_prefix("#x") {
                    u32::from_str_radix(number, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .map(|value| value.to_string())
                } else if let Some(number) = reference.strip_prefix('#') {
                    number
                        .parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .map(|value| value.to_string())
                } else {
                    resolve_predefined_entity(&reference).map(ToOwned::to_owned)
                }
                .ok_or_else(|| format!("XML contains unknown entity &{reference};"))?;
                let parent = stack
                    .last_mut()
                    .ok_or("XML entity appears outside the root element")?;
                parent.children.push(XmlChild::Text(value));
            }
            Event::End(_) => {
                let element = stack.pop().ok_or("XML contains an unmatched close tag")?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(XmlChild::Element(element));
                } else if root.replace(element).is_some() {
                    return Err("XML contains more than one root element".into());
                }
            }
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err("XML document ended before all elements were closed".into());
    }
    Ok(XmlDocument {
        root: root.ok_or("XML document has no root element")?,
    })
}

impl XmlDocument {
    pub(super) fn outer_xml(&self) -> String {
        self.root.outer_xml()
    }

    pub(super) fn select(&self, path: &str) -> Result<Vec<XmlSelection>, String> {
        let path = path.trim();
        if path.is_empty() || path == "." {
            return Ok(vec![XmlSelection {
                element_path: Vec::new(),
                attribute: None,
            }]);
        }
        if path.contains(['|', ':']) {
            return Err(
                "native.xpath.unsupported: namespace and union expressions are unsupported".into(),
            );
        }
        let absolute = path.starts_with('/') && !path.starts_with("//");
        let marked = path.replace("//", "/__DESCENDANT__/");
        let mut descendant = path.starts_with("//");
        let mut steps = Vec::new();
        for part in marked
            .trim_start_matches("./")
            .split('/')
            .filter(|part| !part.is_empty())
        {
            if part == "__DESCENDANT__" {
                descendant = true;
                continue;
            }
            steps.push((descendant, parse_xpath_step(part)?));
            descendant = false;
        }
        if steps.is_empty() {
            return Ok(Vec::new());
        }

        let mut current = vec![Vec::<usize>::new()];
        let mut offset = 0;
        if absolute
            && matches!(&steps[0].1.test, XPathTest::Element(name) if name == "*" || name == &self.root.name)
            && !steps[0].0
        {
            if !predicate_matches(&self.root, steps[0].1.predicate.as_ref()) {
                return Ok(Vec::new());
            }
            offset = 1;
        }
        for (descendant, step) in &steps[offset..] {
            if let XPathTest::Attribute(name) = &step.test {
                if *descendant || step.predicate.is_some() {
                    return Err(
                        "native.xpath.unsupported: attribute axes cannot have predicates".into(),
                    );
                }
                let mut output = Vec::new();
                for path in current {
                    let element = self.element(&path)?;
                    for (index, (candidate, _)) in element.attributes.iter().enumerate() {
                        if name == "*" || candidate == name {
                            output.push(XmlSelection {
                                element_path: path.clone(),
                                attribute: Some(index),
                            });
                        }
                    }
                }
                return Ok(output);
            }
            let XPathTest::Element(name) = &step.test else {
                unreachable!()
            };
            let mut next = Vec::new();
            for path in current {
                let mut candidates = Vec::new();
                if *descendant {
                    self.descendant_paths(&path, name, &mut candidates)?;
                } else {
                    let element = self.element(&path)?;
                    for (index, child) in element.children.iter().enumerate() {
                        if let XmlChild::Element(child) = child
                            && (name == "*" || child.name == *name)
                        {
                            let mut child_path = path.clone();
                            child_path.push(index);
                            candidates.push(child_path);
                        }
                    }
                }
                apply_xpath_predicate(self, &mut candidates, step.predicate.as_ref());
                next.extend(candidates);
            }
            current = next;
        }
        Ok(current
            .into_iter()
            .map(|element_path| XmlSelection {
                element_path,
                attribute: None,
            })
            .collect())
    }

    pub(super) fn selection_value(&self, selection: &XmlSelection, style: i64) -> String {
        let Ok(element) = self.element(&selection.element_path) else {
            return String::new();
        };
        if let Some(attribute) = selection.attribute {
            let Some((name, value)) = element.attributes.get(attribute) else {
                return String::new();
            };
            return match style {
                3 => format!("{name}=\"{}\"", xml_attribute_escape(value)),
                4 => name.clone(),
                _ => value.clone(),
            };
        }
        match style {
            1 => element.inner_text(),
            2 => element.inner_xml(),
            3 => element.outer_xml(),
            4 => element.name.clone(),
            _ => String::new(),
        }
    }

    pub(super) fn element(&self, path: &[usize]) -> Result<&XmlElement, String> {
        let mut element = &self.root;
        for index in path {
            element = match element.children.get(*index) {
                Some(XmlChild::Element(child)) => child,
                _ => return Err("XML selection path became invalid".into()),
            };
        }
        Ok(element)
    }

    pub(super) fn element_mut(&mut self, path: &[usize]) -> Result<&mut XmlElement, String> {
        let mut element = &mut self.root;
        for index in path {
            element = match element.children.get_mut(*index) {
                Some(XmlChild::Element(child)) => child,
                _ => return Err("XML selection path became invalid".into()),
            };
        }
        Ok(element)
    }

    pub(super) fn descendant_paths(
        &self,
        start: &[usize],
        name: &str,
        output: &mut Vec<Vec<usize>>,
    ) -> Result<(), String> {
        let element = self.element(start)?;
        if (name == "*" || element.name == name) && !start.is_empty() {
            output.push(start.to_vec());
        }
        for (index, child) in element.children.iter().enumerate() {
            if matches!(child, XmlChild::Element(_)) {
                let mut path = start.to_vec();
                path.push(index);
                self.descendant_paths(&path, name, output)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn apply_mutation(
        &mut self,
        mutation: XmlMutation,
        request: &NativeCallRequest,
        selections: &[XmlSelection],
    ) -> Result<bool, String> {
        let mut applied = true;
        match mutation {
            XmlMutation::Set => {
                let value = string_argument(request, 2)?.to_owned();
                let style = optional_integer(request, 4)
                    .filter(|value| (0..=2).contains(value))
                    .unwrap_or(0);
                for selection in selections {
                    let element = self.element_mut(&selection.element_path)?;
                    if let Some(attribute) = selection.attribute {
                        if let Some((_, target)) = element.attributes.get_mut(attribute) {
                            target.clone_from(&value);
                        }
                    } else if style == 1 {
                        element.children = vec![XmlChild::Text(value.clone())];
                    } else if style == 2 {
                        element.children = parse_xml_fragment(&value)?;
                    } else {
                        // XmlElement.Value cannot be assigned in System.Xml.
                        return Err("XML_SET style 0 requires an attribute or text node".into());
                    }
                }
            }
            XmlMutation::AddNode => {
                let child = parse_xml(string_argument(request, 2)?)?.root;
                let method = optional_integer(request, 3)
                    .filter(|value| (0..=2).contains(value))
                    .unwrap_or(0);
                for selection in sorted_selections(selections, method != 0) {
                    if selection.attribute.is_some() {
                        applied = false;
                    } else if method == 0 {
                        self.element_mut(&selection.element_path)?
                            .children
                            .push(XmlChild::Element(child.clone()));
                    } else {
                        applied &= insert_sibling(
                            self,
                            &selection.element_path,
                            child.clone(),
                            method == 2,
                        )?;
                    }
                }
            }
            XmlMutation::AddAttribute => {
                let name = string_argument(request, 2)?.to_owned();
                if name.is_empty() || name.contains(['<', '>', '=', '/', ':']) {
                    return Err("XML attribute name is invalid".into());
                }
                let value = optional_string(request, 3).unwrap_or_default().to_owned();
                let method = optional_integer(request, 4)
                    .filter(|value| (0..=2).contains(value))
                    .unwrap_or(0);
                for selection in selections {
                    let element = self.element_mut(&selection.element_path)?;
                    if method == 0 {
                        if selection.attribute.is_none() {
                            element.attributes.push((name.clone(), value.clone()));
                        } else {
                            applied = false;
                        }
                    } else {
                        let Some(index) = selection.attribute else {
                            applied = false;
                            continue;
                        };
                        let insert = index + usize::from(method == 2);
                        element
                            .attributes
                            .insert(insert, (name.clone(), value.clone()));
                    }
                }
            }
            XmlMutation::RemoveNode => {
                for selection in sorted_selections(selections, true) {
                    if selection.attribute.is_some() {
                        applied = false;
                    } else {
                        applied &= remove_element(self, &selection.element_path)?;
                    }
                }
            }
            XmlMutation::RemoveAttribute => {
                let mut selections = selections.to_vec();
                selections.sort_by(|left, right| {
                    right
                        .element_path
                        .cmp(&left.element_path)
                        .then_with(|| right.attribute.cmp(&left.attribute))
                });
                for selection in selections {
                    if let Some(index) = selection.attribute {
                        let element = self.element_mut(&selection.element_path)?;
                        if index < element.attributes.len() {
                            element.attributes.remove(index);
                        }
                    } else {
                        applied = false;
                    }
                }
            }
            XmlMutation::Replace => {
                let replacement = parse_xml(string_argument(request, 2)?)?.root;
                for selection in sorted_selections(selections, true) {
                    if selection.attribute.is_some() {
                        applied = false;
                    } else {
                        applied &=
                            replace_element(self, &selection.element_path, replacement.clone())?;
                    }
                }
            }
        }
        Ok(applied)
    }
}

#[derive(Clone, Debug)]
struct XPathStep {
    test: XPathTest,
    predicate: Option<XPathPredicate>,
}

#[derive(Clone, Debug)]
enum XPathTest {
    Element(String),
    Attribute(String),
}

#[derive(Clone, Debug)]
enum XPathPredicate {
    Position(usize),
    Last,
    AttributeExists(String),
    AttributeEquals(String, String),
    TextEquals(String),
    ChildEquals(String, String),
}

fn parse_xpath_step(value: &str) -> Result<XPathStep, String> {
    let (test, predicate) = if let Some(open) = value.find('[') {
        if !value.ends_with(']') {
            return Err("native.xpath.unsupported: malformed predicate".into());
        }
        (&value[..open], Some(&value[open + 1..value.len() - 1]))
    } else {
        (value, None)
    };
    if test.is_empty() || test.contains(['(', ')']) {
        return Err("native.xpath.unsupported: unsupported node test".into());
    }
    let test = test.strip_prefix('@').map_or_else(
        || XPathTest::Element(test.to_owned()),
        |name| XPathTest::Attribute(name.to_owned()),
    );
    let predicate = predicate.map(parse_xpath_predicate).transpose()?;
    Ok(XPathStep { test, predicate })
}

fn parse_xpath_predicate(value: &str) -> Result<XPathPredicate, String> {
    let value = value.trim();
    if value == "last()" {
        return Ok(XPathPredicate::Last);
    }
    if let Ok(position) = value.parse::<usize>() {
        return Ok(XPathPredicate::Position(position));
    }
    if let Some(attribute) = value.strip_prefix('@') {
        if let Some((name, literal)) = attribute.split_once('=') {
            return Ok(XPathPredicate::AttributeEquals(
                name.trim().to_owned(),
                xpath_literal(literal)?,
            ));
        }
        return Ok(XPathPredicate::AttributeExists(attribute.trim().to_owned()));
    }
    if let Some(literal) = value.strip_prefix("text()=") {
        return Ok(XPathPredicate::TextEquals(xpath_literal(literal)?));
    }
    if let Some((child, literal)) = value.split_once('=')
        && !child.trim().is_empty()
    {
        return Ok(XPathPredicate::ChildEquals(
            child.trim().to_owned(),
            xpath_literal(literal)?,
        ));
    }
    Err("native.xpath.unsupported: predicate is outside the fixed XPath subset".into())
}

pub(super) fn xpath_literal(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
    {
        Ok(value[1..value.len() - 1].to_owned())
    } else {
        Err("native.xpath.unsupported: predicate literal must be quoted".into())
    }
}

fn apply_xpath_predicate(
    document: &XmlDocument,
    candidates: &mut Vec<Vec<usize>>,
    predicate: Option<&XPathPredicate>,
) {
    match predicate {
        None => {}
        Some(XPathPredicate::Position(position)) => {
            let selected = position
                .checked_sub(1)
                .and_then(|index| candidates.get(index))
                .cloned();
            candidates.clear();
            candidates.extend(selected);
        }
        Some(XPathPredicate::Last) => {
            let selected = candidates.last().cloned();
            candidates.clear();
            candidates.extend(selected);
        }
        Some(predicate) => candidates.retain(|path| {
            document
                .element(path)
                .is_ok_and(|element| predicate_matches(element, Some(predicate)))
        }),
    }
}

fn predicate_matches(element: &XmlElement, predicate: Option<&XPathPredicate>) -> bool {
    match predicate {
        None | Some(XPathPredicate::Position(1) | XPathPredicate::Last) => true,
        Some(XPathPredicate::Position(_)) => false,
        Some(XPathPredicate::AttributeExists(name)) => element
            .attributes
            .iter()
            .any(|(candidate, _)| candidate == name),
        Some(XPathPredicate::AttributeEquals(name, value)) => element
            .attributes
            .iter()
            .any(|(candidate, candidate_value)| candidate == name && candidate_value == value),
        Some(XPathPredicate::TextEquals(value)) => element.inner_text() == *value,
        Some(XPathPredicate::ChildEquals(name, value)) => element
            .elements_named(name)
            .iter()
            .any(|child| child.inner_text() == *value),
    }
}

pub(super) fn sorted_selections(selections: &[XmlSelection], reverse: bool) -> Vec<XmlSelection> {
    let mut result = selections.to_vec();
    if reverse {
        result.sort_by(|left, right| right.element_path.cmp(&left.element_path));
    }
    result
}

pub(super) fn insert_sibling(
    document: &mut XmlDocument,
    path: &[usize],
    child: XmlElement,
    after: bool,
) -> Result<bool, String> {
    let Some((index, parent)) = path.split_last() else {
        return Ok(false);
    };
    let parent = document.element_mut(parent)?;
    parent
        .children
        .insert(*index + usize::from(after), XmlChild::Element(child));
    Ok(true)
}

pub(super) fn remove_element(document: &mut XmlDocument, path: &[usize]) -> Result<bool, String> {
    let Some((index, parent)) = path.split_last() else {
        return Ok(false);
    };
    let parent = document.element_mut(parent)?;
    if *index < parent.children.len() {
        parent.children.remove(*index);
    }
    Ok(true)
}

pub(super) fn replace_element(
    document: &mut XmlDocument,
    path: &[usize],
    replacement: XmlElement,
) -> Result<bool, String> {
    let Some((index, parent)) = path.split_last() else {
        return Ok(false);
    };
    let parent = document.element_mut(parent)?;
    let Some(slot) = parent.children.get_mut(*index) else {
        return Err("XML replacement path became invalid".into());
    };
    *slot = XmlChild::Element(replacement);
    Ok(true)
}

pub(super) fn parse_xml_fragment(value: &str) -> Result<Vec<XmlChild>, String> {
    Ok(parse_xml(&format!(
        "<__rustyera_fragment>{value}</__rustyera_fragment>"
    ))?
    .root
    .children)
}

impl XmlElement {
    pub(super) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find_map(|(candidate, value)| (candidate == name).then_some(value.as_str()))
    }

    pub(super) fn elements_named(&self, name: &str) -> Vec<&Self> {
        self.children
            .iter()
            .filter_map(|child| match child {
                XmlChild::Element(element) if name == "*" || element.name == name => Some(element),
                XmlChild::Element(_) | XmlChild::Text(_) => None,
            })
            .collect()
    }

    pub(super) fn inner_text(&self) -> String {
        let mut output = String::new();
        for child in &self.children {
            match child {
                XmlChild::Text(value) => output.push_str(value),
                XmlChild::Element(element) => output.push_str(&element.inner_text()),
            }
        }
        output
    }

    pub(super) fn inner_xml(&self) -> String {
        let mut output = String::new();
        for child in &self.children {
            match child {
                XmlChild::Text(value) => output.push_str(&xml_text_escape(value)),
                XmlChild::Element(element) => output.push_str(&element.outer_xml()),
            }
        }
        output
    }

    pub(super) fn outer_xml(&self) -> String {
        let mut output = String::new();
        output.push('<');
        output.push_str(&self.name);
        for (name, value) in &self.attributes {
            output.push(' ');
            output.push_str(name);
            output.push_str("=\"");
            output.push_str(&xml_attribute_escape(value));
            output.push('"');
        }
        if self.children.is_empty() {
            output.push_str(" />");
        } else {
            output.push('>');
            output.push_str(&self.inner_xml());
            output.push_str("</");
            output.push_str(&self.name);
            output.push('>');
        }
        output
    }
}

pub(super) fn xml_text_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(super) fn xml_attribute_escape(value: &str) -> String {
    xml_text_escape(value).replace('"', "&quot;")
}
