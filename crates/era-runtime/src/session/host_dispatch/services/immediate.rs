#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Copy)]
struct ImmediateTextFormatting<'a> {
    bar_char_1: char,
    bar_char_2: char,
    money_first: bool,
    money_label: &'a str,
}

#[derive(Clone)]
pub(in crate::session) struct ImmediateTagSplitTargets {
    result: PlaceDescriptor,
    results: PlaceDescriptor,
    results_capacity: usize,
}

pub(in crate::session) struct ImmediateRuntimeHost<'a> {
    presentation: &'a mut PresentationModel,
    pending_presentation_update: &'a mut bool,
    command_intents: &'a mut BTreeMap<InteractionToken, VmValue>,
    next_interaction_id: &'a mut u64,
    epoch: u64,
    button_generation: u64,
    line_count_place: Option<PlaceDescriptor>,
    query_state: RuntimeQueryState,
    user_defined_skip: bool,
    force_kana_mode: u8,
    text_formatting: Option<ImmediateTextFormatting<'a>>,
    tag_split_targets: Option<ImmediateTagSplitTargets>,
}

impl VmHost for ImmediateRuntimeHost<'_> {
    fn path_memo_safe(&self, import: &erabasic_bytecode::RuntimeImport) -> bool {
        immediate_host_path_memo_safe(&import.namespace, &import.name)
    }

    fn call_immediate(&mut self, request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
        let name = request.normalized_name;
        if skips_runtime_command_immediately(
            &request.import.import.namespace,
            name,
            self.query_state.skip_print,
            self.user_defined_skip,
        ) {
            return ImmediateHostCallResult::Ready(HostReady::empty());
        }
        if !request
            .import
            .import
            .namespace
            .eq_ignore_ascii_case("rustyera.text")
        {
            return ImmediateHostCallResult::Unsupported;
        }
        if name == "HTML_TAGSPLIT"
            && let Some(ready) =
                immediate_html_tag_split(request.arguments, self.tag_split_targets.as_ref())
        {
            return ImmediateHostCallResult::Ready(ready);
        }
        if let Some(value) = immediate_text_value(name, request.arguments, self.text_formatting) {
            return ImmediateHostCallResult::Ready(HostReady {
                value: Some(value),
                writes: Vec::new(),
            });
        }
        if let Ok(RuntimeQueryEvaluation::Ready(value)) =
            evaluate_runtime_query(name, request.arguments, self.presentation, self.query_state)
        {
            return ImmediateHostCallResult::Ready(HostReady {
                value: Some(value),
                writes: Vec::new(),
            });
        }
        if let Ok(Some(prepared)) = PreparedPresentationState::prepare(name, request.arguments) {
            prepared.apply(self.presentation);
            return ImmediateHostCallResult::Ready(HostReady::empty());
        }
        if let Ok(button) = PreparedButton::prepare(name, request.arguments) {
            let token = self.allocate_interaction();
            let value = button.apply(self.presentation, token);
            self.command_intents.insert(token, value);
            *self.pending_presentation_update = true;
            return ImmediateHostCallResult::Ready(HostReady::empty());
        }
        if let Some(ready) = self.immediate_line_edit(name, request.arguments) {
            return ImmediateHostCallResult::Ready(ready);
        }
        if name == "HTML_PRINT"
            && let Some(ready) = self.immediate_html_print(request.arguments)
        {
            return ImmediateHostCallResult::Ready(ready);
        }
        if matches!(name, "HTML_PRINTC" | "HTML_PRINTLC")
            && let Some(ready) = self.immediate_html_column_print(name, request.arguments)
        {
            return ImmediateHostCallResult::Ready(ready);
        }
        let commits_line = is_immediate_committed_text_print(name);
        if !is_immediate_text_print(name) && !commits_line {
            return ImmediateHostCallResult::Unsupported;
        }
        if self.query_state.skip_print {
            return ImmediateHostCallResult::Ready(HostReady::empty());
        }
        let prepared = PreparedGenericPrint::prepare(name, request.arguments, self.force_kana_mode);
        if !prepared.is_immediate_safe() {
            return ImmediateHostCallResult::Unsupported;
        }
        let ready = if commits_line {
            prepared.apply_committed(self, name)
        } else {
            prepared.apply_uncommitted(self.presentation, name);
            HostReady::empty()
        };
        *self.pending_presentation_update = true;
        ImmediateHostCallResult::Ready(ready)
    }

    fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
        HostCallResult::Error("immediate presentation host cannot capture deferred calls".into())
    }
}

fn immediate_host_path_memo_safe(namespace: &str, name: &str) -> bool {
    namespace.eq_ignore_ascii_case("rustyera.text")
        && ["HTML_ESCAPE", "HTML_TAGSPLIT", "HTML_TOPLAINTEXT", "TOSTR"]
            .iter()
            .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

struct PreparedButton {
    text: String,
    value: VmValue,
    protocol_value: era_runtime_protocol::ProtocolValue,
    alignment: Option<CellAlignment>,
}

#[derive(Clone, Copy, Debug)]
enum ButtonPreparationError {
    Unsupported,
    MissingValue,
    UnmaterializedValue,
}

impl PreparedButton {
    fn prepare(name: &str, arguments: &[VmValue]) -> Result<Self, ButtonPreparationError> {
        if !matches!(name, "PRINTBUTTON" | "PRINTBUTTONC" | "PRINTBUTTONLC") {
            return Err(ButtonPreparationError::Unsupported);
        }
        let value = arguments
            .get(1)
            .ok_or(ButtonPreparationError::MissingValue)?
            .clone();
        let protocol_value = materialized_protocol_value(&value)
            .ok_or(ButtonPreparationError::UnmaterializedValue)?;
        let alignment = match name {
            "PRINTBUTTONC" => Some(CellAlignment::Right),
            "PRINTBUTTONLC" => Some(CellAlignment::Left),
            _ => None,
        };
        Ok(Self {
            text: arguments
                .first()
                .map_or_else(String::new, display_value)
                .replace('\n', ""),
            value,
            protocol_value,
            alignment,
        })
    }

    fn apply(self, presentation: &mut PresentationModel, token: InteractionToken) -> VmValue {
        presentation.append_button(self.text, self.protocol_value, token, self.alignment);
        self.value
    }
}

impl RuntimeSession {
    pub(in crate::session) fn immediate_runtime_host(
        &mut self,
        bar_char_1: char,
        bar_char_2: char,
        line_count_place: Option<PlaceDescriptor>,
        tag_split_targets: Option<ImmediateTagSplitTargets>,
    ) -> ImmediateRuntimeHost<'_> {
        let text_formatting =
            self.project_snapshot
                .as_ref()
                .map(|project| ImmediateTextFormatting {
                    bar_char_1,
                    bar_char_2,
                    money_first: project.money_first,
                    money_label: project.money_label.as_str(),
                });
        ImmediateRuntimeHost {
            presentation: &mut self.presentation,
            pending_presentation_update: &mut self.pending_presentation_update,
            command_intents: &mut self.command_intents,
            next_interaction_id: &mut self.next_interaction_id,
            epoch: self.epoch.0,
            button_generation: self.button_generation,
            line_count_place,
            query_state: RuntimeQueryState {
                skip_print: self.skip_print,
                message_skip: self.message_skip,
                snake_display_state: self.project_snapshot.as_ref().is_some_and(|project| {
                    project
                        .manifest
                        .compatibility
                        .supports_snake_display_state()
                }),
            },
            user_defined_skip: self.user_defined_skip,
            force_kana_mode: self.force_kana_mode,
            text_formatting,
            tag_split_targets,
        }
    }
}

pub(in crate::session) fn immediate_tag_split_targets(
    vm: &RuntimeVm,
) -> Option<ImmediateTagSplitTargets> {
    let count_definition = vm.vm().global_by_name("RESULT")?;
    let [count_capacity] = count_definition.dimensions.as_slice() else {
        return None;
    };
    if count_definition.value_type != erabasic_bytecode::BytecodeType::Integer
        || *count_capacity == 0
        || !count_definition.mutable
    {
        return None;
    }
    let tokens_definition = vm.vm().global_by_name("RESULTS")?;
    let [tokens_capacity] = tokens_definition.dimensions.as_slice() else {
        return None;
    };
    if tokens_definition.value_type != erabasic_bytecode::BytecodeType::String
        || !tokens_definition.mutable
    {
        return None;
    }
    Some(ImmediateTagSplitTargets {
        result: global_place_at(vm, "RESULT", 0)?,
        results: global_place_at(vm, "RESULTS", 0)?,
        results_capacity: usize::try_from(*tokens_capacity).unwrap_or(0),
    })
}

fn immediate_html_tag_split(
    arguments: &[VmValue],
    targets: Option<&ImmediateTagSplitTargets>,
) -> Option<HostReady> {
    let [VmValue::String(source)] = arguments else {
        return None;
    };
    let targets = targets?;
    let Ok(values) = split_html_tags(source) else {
        return Some(HostReady {
            value: None,
            writes: vec![HostWrite {
                target: targets.result.clone(),
                value: VmValue::Integer(-1),
            }],
        });
    };
    let mut writes = values
        .iter()
        .take(targets.results_capacity)
        .enumerate()
        .map(|(index, value)| {
            let mut target = targets.results.clone();
            target.indices[0] = u64::try_from(index).unwrap_or(u64::MAX);
            HostWrite {
                target,
                value: VmValue::String(value.clone()),
            }
        })
        .collect::<Vec<_>>();
    writes.push(HostWrite {
        target: targets.result.clone(),
        value: VmValue::Integer(i64::try_from(values.len()).unwrap_or(i64::MAX)),
    });
    Some(HostReady {
        value: None,
        writes,
    })
}

fn skips_runtime_command_immediately(
    namespace: &str,
    name: &str,
    skip_print: bool,
    user_defined_skip: bool,
) -> bool {
    skip_print
        && !namespace.eq_ignore_ascii_case("rustyera.extension")
        && is_runtime_print_command(name)
        && !(user_defined_skip && is_input_command(name))
}

fn immediate_text_value(
    name: &str,
    arguments: &[VmValue],
    formatting: Option<ImmediateTextFormatting<'_>>,
) -> Option<VmValue> {
    match name {
        "TOSTR" | "MONEYSTR" => {
            let VmValue::Integer(value) = arguments.first()? else {
                return None;
            };
            let format = match arguments.get(1) {
                None => None,
                Some(VmValue::String(format)) => Some(format.as_str()),
                Some(_) => return None,
            };
            if name == "TOSTR" {
                return Some(VmValue::String(
                    format_optional_era_integer(*value, format).ok()?,
                ));
            }
            let formatted = format_optional_era_integer(*value, format).ok()?;
            let formatting = formatting?;
            Some(VmValue::String(decorate_money_value(
                &formatted,
                formatting.money_first,
                formatting.money_label,
            )))
        }
        "TOFULL" | "TOHALF" => {
            let VmValue::String(value) = arguments.first()? else {
                return None;
            };
            Some(VmValue::String(if name == "TOFULL" {
                to_full_width(value)
            } else {
                to_half_width(value)
            }))
        }
        "BARSTR" => {
            let formatting = formatting?;
            let [
                VmValue::Integer(value),
                VmValue::Integer(maximum),
                VmValue::Integer(length),
                ..,
            ] = arguments
            else {
                return None;
            };
            format_bar_string(
                *value,
                *maximum,
                *length,
                formatting.bar_char_1,
                formatting.bar_char_2,
            )
            .ok()
            .map(VmValue::String)
        }
        _ => None,
    }
}

fn is_immediate_text_print(name: &str) -> bool {
    if name == "PRINTPLAIN" {
        return true;
    }
    let stem = name.trim_end_matches(['K', 'D', 'N']);
    matches!(
        stem,
        "PRINT"
            | "PRINTV"
            | "PRINTS"
            | "PRINTFORM"
            | "PRINTFORMS"
            | "PRINTPLAIN"
            | "PRINTPLAINFORM"
            | "PRINTSINGLE"
            | "PRINTSINGLEV"
            | "PRINTSINGLES"
            | "PRINTSINGLEFORM"
            | "PRINTSINGLEFORMS"
    ) && !print_commits_line(name)
}

fn is_immediate_committed_text_print(name: &str) -> bool {
    if name.ends_with('W') || !print_commits_line(name) || column_print_alignment(name).is_some() {
        return false;
    }
    let stem = name.trim_end_matches(['K', 'D', 'N', 'L']);
    matches!(
        stem,
        "PRINT"
            | "PRINTV"
            | "PRINTS"
            | "PRINTFORM"
            | "PRINTFORMS"
            | "PRINTPLAIN"
            | "PRINTPLAINFORM"
            | "PRINTSINGLE"
            | "PRINTSINGLEV"
            | "PRINTSINGLES"
            | "PRINTSINGLEFORM"
            | "PRINTSINGLEFORMS"
    )
}

struct PreparedGenericPrint {
    text: String,
}

impl PreparedGenericPrint {
    fn prepare(name: &str, arguments: &[VmValue], force_kana_mode: u8) -> Self {
        let mut text = arguments.iter().map(display_value).collect::<String>();
        if print_uses_kana_conversion(name) {
            text = convert_kana_mode(&text, force_kana_mode);
        }
        Self { text }
    }

    fn is_immediate_safe(&self) -> bool {
        !self.text.contains('\n')
    }

    fn apply_uncommitted(self, presentation: &mut PresentationModel, name: &str) {
        if print_uses_default_color(name) {
            presentation.append_default_color_text(self.text, false, false);
        } else if name.starts_with("PRINTPLAIN") {
            presentation.append_plain_print_text(self.text, false, false);
        } else {
            presentation.append_print_text(self.text, false, false);
        }
    }

    fn apply_committed(self, host: &mut ImmediateRuntimeHost<'_>, name: &str) -> HostReady {
        let default_color = print_uses_default_color(name);
        let plain = name.starts_with("PRINTPLAIN");
        if default_color {
            host.presentation
                .append_default_color_text(self.text, false, false);
            host.presentation.force_default_color_new_line();
        } else if plain {
            host.presentation
                .append_plain_print_text(self.text, false, false);
            host.presentation.force_new_line();
        } else {
            host.presentation.append_print_text(self.text, false, false);
            host.presentation.force_new_line();
        }
        if !plain {
            host.bind_last_output_buttons();
        }
        host.complete_line_count()
    }
}

impl ImmediateRuntimeHost<'_> {
    fn immediate_line_edit(&mut self, name: &str, arguments: &[VmValue]) -> Option<HostReady> {
        let prepared = PreparedLineEdit::prepare(name, arguments)?;
        prepared.apply(self.presentation);
        *self.pending_presentation_update = true;
        Some(self.complete_line_count())
    }

    fn immediate_html_print(&mut self, arguments: &[VmValue]) -> Option<HostReady> {
        let Ok(mut prepared) = PreparedHtmlPrint::prepare(arguments) else {
            return None;
        };
        if (!self.query_state.snake_display_state
            && erabasic_html::snake_extension_range(&prepared.document).is_some())
            || !prepared.warnings.is_empty()
            || document_has_unresolved_color_matrix(&prepared.document)
        {
            return None;
        }
        let mut bindings = HtmlInteractionBindings {
            epoch: self.epoch,
            next_interaction_id: self.next_interaction_id,
            button_generation: self.button_generation,
            command_intents: self.command_intents,
        };
        bind_html_document(&mut bindings, &mut prepared.document);
        prepared.apply(self.presentation);
        *self.pending_presentation_update = true;
        Some(self.complete_line_count())
    }

    fn immediate_html_column_print(
        &mut self,
        name: &str,
        arguments: &[VmValue],
    ) -> Option<HostReady> {
        if !self.query_state.snake_display_state {
            return None;
        }
        let Ok(mut prepared) = PreparedHtmlColumnPrint::prepare(name, arguments) else {
            return None;
        };
        if !prepared.warnings.is_empty() || document_has_unresolved_color_matrix(&prepared.document)
        {
            return None;
        }
        let mut bindings = HtmlInteractionBindings {
            epoch: self.epoch,
            next_interaction_id: self.next_interaction_id,
            button_generation: self.button_generation,
            command_intents: self.command_intents,
        };
        bind_html_document(&mut bindings, &mut prepared.document);
        if prepared.apply(self.presentation) {
            *self.pending_presentation_update = true;
        }
        Some(self.complete_line_count())
    }

    fn complete_line_count(&mut self) -> HostReady {
        let writes = self
            .line_count_place
            .clone()
            .map_or_else(Vec::new, |target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(self.presentation.logical_line_count()),
                }]
            });
        self.presentation.mark_line_count_synchronized();
        HostReady {
            value: None,
            writes,
        }
    }

    fn allocate_interaction(&mut self) -> InteractionToken {
        next_interaction_token(self.epoch, self.next_interaction_id)
    }

    fn bind_last_output_buttons(&mut self) {
        let count = self.presentation.last_line_auto_button_values().len();
        let tokens = (0..count)
            .map(|_| self.allocate_interaction())
            .collect::<Vec<_>>();
        for (token, value) in self.presentation.bind_last_line_auto_buttons(&tokens) {
            self.command_intents.insert(token, VmValue::Integer(value));
        }
    }
}
