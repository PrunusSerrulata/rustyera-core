//! Portable XML tree, mutation, and the supported deterministic `XPath` subset.

use super::{
    Event, NativeCallRequest, Reader, XmlChild, XmlDocument, XmlElement, XmlMutation, XmlSelection,
    optional_integer, optional_string, resolve_predefined_entity, string_argument,
};
use crate::ExecutionFailure;
use crate::structured::{argument_failure, parse_failure};

mod xpath;

#[allow(clippy::too_many_lines)]
pub(super) fn parse_xml(input: &str) -> Result<XmlDocument, ExecutionFailure> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut stack = Vec::<XmlElement>::new();
    let mut root = None;
    loop {
        match reader
            .read_event()
            .map_err(|error| parse_failure(error.to_string()))?
        {
            Event::Start(start) => {
                let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                let attributes = start
                    .attributes()
                    .map(|attribute| {
                        let attribute =
                            attribute.map_err(|error| parse_failure(error.to_string()))?;
                        Ok((
                            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                            attribute
                                .decode_and_unescape_value(reader.decoder())
                                .map_err(|error| parse_failure(error.to_string()))?
                                .into_owned(),
                        ))
                    })
                    .collect::<Result<Vec<_>, ExecutionFailure>>()?;
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
                            let attribute =
                                attribute.map_err(|error| parse_failure(error.to_string()))?;
                            Ok((
                                String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                                attribute
                                    .decode_and_unescape_value(reader.decoder())
                                    .map_err(|error| parse_failure(error.to_string()))?
                                    .into_owned(),
                            ))
                        })
                        .collect::<Result<Vec<_>, ExecutionFailure>>()?,
                    children: Vec::new(),
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(XmlChild::Element(element));
                } else if root.replace(element).is_some() {
                    return Err(parse_failure("XML contains more than one root element"));
                }
            }
            Event::Text(text) => {
                let value = text
                    .decode()
                    .map_err(|error| parse_failure(error.to_string()))?;
                let value = quick_xml::escape::unescape(&value)
                    .map_err(|error| parse_failure(error.to_string()))?
                    .into_owned();
                if let Some(parent) = stack.last_mut() {
                    // XmlDocument.PreserveWhitespace defaults to false in the
                    // fixed Emuera reference. eraFL XML files are indented,
                    // so formatting-only nodes must not leak into InnerXml or
                    // mutation output.
                    if !value.trim().is_empty() {
                        parent.children.push(XmlChild::Text(value));
                    }
                } else if !value.trim().is_empty() {
                    return Err(parse_failure("XML text appears outside the root element"));
                }
            }
            Event::CData(text) => {
                let value = text
                    .decode()
                    .map_err(|error| parse_failure(error.to_string()))?
                    .into_owned();
                let parent = stack
                    .last_mut()
                    .ok_or_else(|| parse_failure("XML CDATA appears outside the root element"))?;
                parent.children.push(XmlChild::Text(value));
            }
            Event::GeneralRef(reference) => {
                let reference = reference
                    .decode()
                    .map_err(|error| parse_failure(error.to_string()))?;
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
                .ok_or_else(|| {
                    parse_failure(format!("XML contains unknown entity &{reference};"))
                })?;
                let parent = stack
                    .last_mut()
                    .ok_or_else(|| parse_failure("XML entity appears outside the root element"))?;
                parent.children.push(XmlChild::Text(value));
            }
            Event::End(_) => {
                let element = stack
                    .pop()
                    .ok_or_else(|| parse_failure("XML contains an unmatched close tag"))?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(XmlChild::Element(element));
                } else if root.replace(element).is_some() {
                    return Err(parse_failure("XML contains more than one root element"));
                }
            }
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err(parse_failure(
            "XML document ended before all elements were closed",
        ));
    }
    Ok(XmlDocument {
        root: root.ok_or_else(|| parse_failure("XML document has no root element"))?,
    })
}

impl XmlDocument {
    pub(super) fn outer_xml(&self) -> String {
        self.root.outer_xml()
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

    pub(super) fn element(&self, path: &[usize]) -> Result<&XmlElement, ExecutionFailure> {
        let mut element = &self.root;
        for index in path {
            element = match element.children.get(*index) {
                Some(XmlChild::Element(child)) => child,
                _ => return Err("XML selection path became invalid".into()),
            };
        }
        Ok(element)
    }

    pub(super) fn element_mut(
        &mut self,
        path: &[usize],
    ) -> Result<&mut XmlElement, ExecutionFailure> {
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
    ) -> Result<(), ExecutionFailure> {
        let element = self.element(start)?;
        for (index, child) in element.children.iter().enumerate() {
            if matches!(child, XmlChild::Element(_)) {
                let mut path = start.to_vec();
                path.push(index);
                self.descendant_or_self_paths(&path, name, output)?;
            }
        }
        Ok(())
    }

    fn descendant_or_self_paths(
        &self,
        start: &[usize],
        name: &str,
        output: &mut Vec<Vec<usize>>,
    ) -> Result<(), ExecutionFailure> {
        let element = self.element(start)?;
        if name == "*" || element.name == name {
            output.push(start.to_vec());
        }
        for (index, child) in element.children.iter().enumerate() {
            if matches!(child, XmlChild::Element(_)) {
                let mut path = start.to_vec();
                path.push(index);
                self.descendant_or_self_paths(&path, name, output)?;
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
    ) -> Result<bool, ExecutionFailure> {
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
                        return Err(argument_failure(
                            "XML_SET style 0 requires an attribute or text node",
                        ));
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
                    return Err(argument_failure("XML attribute name is invalid"));
                }
                let value = optional_string(request, 3).unwrap_or_default().to_owned();
                let method = optional_integer(request, 4)
                    .filter(|value| (0..=2).contains(value))
                    .unwrap_or(0);
                for selection in selections {
                    let element = self.element_mut(&selection.element_path)?;
                    if method == 0 {
                        if selection.attribute.is_none() {
                            element.append_attribute(name.clone(), value.clone());
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
) -> Result<bool, ExecutionFailure> {
    let Some((index, parent)) = path.split_last() else {
        return Ok(false);
    };
    let parent = document.element_mut(parent)?;
    parent
        .children
        .insert(*index + usize::from(after), XmlChild::Element(child));
    Ok(true)
}

pub(super) fn remove_element(
    document: &mut XmlDocument,
    path: &[usize],
) -> Result<bool, ExecutionFailure> {
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
) -> Result<bool, ExecutionFailure> {
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

pub(super) fn parse_xml_fragment(value: &str) -> Result<Vec<XmlChild>, ExecutionFailure> {
    Ok(parse_xml(&format!(
        "<__rustyera_fragment>{value}</__rustyera_fragment>"
    ))?
    .root
    .children)
}

impl XmlElement {
    pub(super) fn append_attribute(&mut self, name: String, value: String) {
        if let Some(index) = self
            .attributes
            .iter()
            .position(|(candidate, _)| candidate == &name)
        {
            self.attributes.remove(index);
        }
        self.attributes.push((name, value));
    }

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
