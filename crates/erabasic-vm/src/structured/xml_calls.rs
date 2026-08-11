#[allow(clippy::wildcard_imports)]
use super::*;

impl StructuredState {
    pub(super) fn call_xml(
        &mut self,
        name: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, String> {
        match name {
            "xml_document" => {
                let id = argument_key(request, 0)?;
                if self.xml_documents.contains_key(&id) {
                    return ready_integer(0);
                }
                let document = parse_xml(string_argument(request, 1)?)?;
                self.xml_documents.insert(id, document);
                ready_integer(1)
            }
            "xml_exist" => {
                let id = argument_key(request, 0)?;
                ready_integer(i64::from(self.xml_documents.contains_key(&id)))
            }
            "xml_release" => {
                let id = argument_key(request, 0)?;
                if self.xml_documents.remove(&id).is_some() {
                    ready_integer(1)
                } else {
                    ready_integer(0)
                }
            }
            "xml_tostr" => Ok(NativeReady::value(VmValue::String(
                self.xml_documents
                    .get(&argument_key(request, 0)?)
                    .map_or_else(String::new, XmlDocument::outer_xml),
            ))),
            "xml_get" | "xml_get_byname" => {
                let inline;
                let document = if name == "xml_get_byname"
                    || matches!(request.arguments.first(), Some(VmValue::Integer(_)))
                {
                    let id = argument_key(request, 0)?;
                    let Some(document) = self.xml_documents.get(&id) else {
                        return ready_integer(-1);
                    };
                    document
                } else {
                    inline = parse_xml(xml_target_string(request, 0)?)?;
                    &inline
                };
                let selected = document.select(string_argument(request, 1)?)?;
                let style = optional_integer(request, 3).unwrap_or(0);
                let values = selected
                    .iter()
                    .map(|selection| document.selection_value(selection, style))
                    .collect::<Vec<_>>();
                let mut writes = Vec::new();
                if request.arguments.len() >= 3 {
                    let target = if matches!(request.arguments.get(2), Some(VmValue::Integer(value)) if *value != 0)
                    {
                        Some(implicit_place(request, "RESULTS")?)
                    } else {
                        request
                            .places
                            .iter()
                            .find(|place| place.argument_index == 2)
                    };
                    if let Some(target) = target {
                        writes.extend(array_writes(
                            target,
                            0,
                            values.into_iter().map(VmValue::String),
                        ));
                    }
                }
                Ok(NativeReady {
                    value: Some(VmValue::Integer(
                        i64::try_from(selected.len()).unwrap_or(i64::MAX),
                    )),
                    writes,
                })
            }
            "xml_set" | "xml_set_byname" => {
                self.mutate_xml(name.ends_with("_byname"), request, XmlMutation::Set)
            }
            "xml_addnode" | "xml_addnode_byname" => {
                self.mutate_xml(name.ends_with("_byname"), request, XmlMutation::AddNode)
            }
            "xml_addattribute" | "xml_addattribute_byname" => self.mutate_xml(
                name.ends_with("_byname"),
                request,
                XmlMutation::AddAttribute,
            ),
            "xml_removenode" | "xml_removenode_byname" => {
                self.mutate_xml(name.ends_with("_byname"), request, XmlMutation::RemoveNode)
            }
            "xml_removeattribute" | "xml_removeattribute_byname" => self.mutate_xml(
                name.ends_with("_byname"),
                request,
                XmlMutation::RemoveAttribute,
            ),
            "xml_replace" | "xml_replace_byname" => {
                self.mutate_xml(name.ends_with("_byname"), request, XmlMutation::Replace)
            }
            _ => Err(format!(
                "XML operation {name} is outside the pinned XPath mutation subset"
            )),
        }
    }

    fn mutate_xml(
        &mut self,
        by_name: bool,
        request: &NativeCallRequest,
        mutation: XmlMutation,
    ) -> Result<NativeReady, String> {
        let target = if by_name
            || matches!(request.arguments.first(), Some(VmValue::Integer(_)))
            || mutation == XmlMutation::Replace && request.arguments.len() == 2
        {
            XmlTarget::Stored(xml_target_key(request, 0)?)
        } else {
            XmlTarget::Inline(explicit_place(request, 0)?.target.clone())
        };
        let mut candidate = match &target {
            XmlTarget::Stored(id) => {
                let Some(document) = self.xml_documents.get(id) else {
                    return ready_integer(-1);
                };
                document.clone()
            }
            XmlTarget::Inline(_) => parse_xml(xml_target_string(request, 0)?)?,
        };
        if mutation == XmlMutation::Replace && request.arguments.len() == 2 {
            let replacement = parse_xml(string_argument(request, 1)?)?;
            let XmlTarget::Stored(id) = target else {
                unreachable!("two-argument XML_REPLACE always resolves a stored document")
            };
            self.xml_documents.insert(id, replacement);
            return ready_integer(1);
        }
        let selected = candidate.select(string_argument(request, 1)?)?;
        let selected_count = selected.len();
        if selected_count == 0 {
            // System.Xml only assigns an inline document back to argument 0
            // from inside its `nodes.Count > 0` branch. In particular, a
            // no-match eraFL mutation must not normalize the caller's XML.
            if mutation == XmlMutation::Replace {
                parse_xml(string_argument(request, 2)?)?;
            }
            return ready_integer(0);
        }
        let set_all_index = match mutation {
            XmlMutation::Set | XmlMutation::Replace => 3,
            XmlMutation::AddNode => 4,
            XmlMutation::AddAttribute => 5,
            XmlMutation::RemoveNode | XmlMutation::RemoveAttribute => 2,
        };
        let set_all = optional_integer(request, set_all_index).is_some_and(|value| value != 0);
        let apply = if selected_count <= 1 || set_all {
            selected
        } else {
            Vec::new()
        };
        let applied = candidate.apply_mutation(mutation, request, &apply)?;
        if selected_count == 1 && !applied {
            return ready_integer(0);
        }
        let writes = match target {
            XmlTarget::Stored(id) => {
                self.xml_documents.insert(id, candidate);
                Vec::new()
            }
            XmlTarget::Inline(target) => vec![HostWrite {
                target,
                value: VmValue::String(candidate.outer_xml()),
            }],
        };
        Ok(NativeReady {
            value: Some(VmValue::Integer(
                i64::try_from(selected_count).unwrap_or(i64::MAX),
            )),
            writes,
        })
    }
}
