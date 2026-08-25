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
    line_count_place: Option<PlaceDescriptor>,
    query_state: RuntimeQueryState,
    user_defined_skip: bool,
    force_kana_mode: u8,
    text_formatting: Option<ImmediateTextFormatting<'a>>,
    tag_split_targets: Option<ImmediateTagSplitTargets>,
}

impl VmHost for ImmediateRuntimeHost<'_> {
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
            line_count_place,
            query_state: RuntimeQueryState {
                skip_print: self.skip_print,
                message_skip: self.message_skip,
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
        let writes = host
            .line_count_place
            .clone()
            .map_or_else(Vec::new, |target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(host.presentation.logical_line_count()),
                }]
            });
        host.presentation.mark_line_count_synchronized();
        HostReady {
            value: None,
            writes,
        }
    }
}

impl ImmediateRuntimeHost<'_> {
    fn allocate_interaction(&mut self) -> InteractionToken {
        let token = InteractionToken {
            epoch: self.epoch,
            id: *self.next_interaction_id,
        };
        *self.next_interaction_id = (*self.next_interaction_id).saturating_add(1);
        token
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

#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn dispatch_services(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
    ) -> Result<(), RuntimeError> {
        if matches!(
            name.as_str(),
            "PRINTBUTTON" | "PRINTBUTTONC" | "PRINTBUTTONLC"
        ) {
            let text = request
                .arguments
                .first()
                .map_or_else(String::new, display_value)
                .replace('\n', "");
            let value = request
                .arguments
                .get(1)
                .cloned()
                .ok_or_else(|| RuntimeError::Internal("PRINTBUTTON value is missing".into()))?;
            let token = self.allocate_interaction();
            let alignment = match name.as_str() {
                "PRINTBUTTONC" => Some(CellAlignment::Right),
                "PRINTBUTTONLC" => Some(CellAlignment::Left),
                _ => None,
            };
            let protocol_value = match &value {
                VmValue::Integer(value) => era_runtime_protocol::ProtocolValue::Integer(*value),
                VmValue::String(value) => {
                    era_runtime_protocol::ProtocolValue::String(value.clone())
                }
                VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                    return Err(RuntimeError::Internal(
                        "PRINTBUTTON value was not materialized".into(),
                    ));
                }
            };
            self.presentation
                .append_button(text, protocol_value, token, alignment);
            self.command_intents.insert(token, value);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(
            name.as_str(),
            "PRINT_ABL" | "PRINT_TALENT" | "PRINT_MARK" | "PRINT_EXP"
        ) {
            let target = u64::try_from(integer_argument_value(&request.arguments, 0)?)
                .map_err(|_| RuntimeError::Internal("character index is negative".into()))?;
            let (variable, table, format) = match name.as_str() {
                "PRINT_ABL" => ("ABL", erabasic_data::NameTableKind::Abl, 0),
                "PRINT_TALENT" => ("TALENT", erabasic_data::NameTableKind::Talent, 1),
                "PRINT_MARK" => ("MARK", erabasic_data::NameTableKind::Mark, 0),
                "PRINT_EXP" => ("EXP", erabasic_data::NameTableKind::Exp, 2),
                _ => unreachable!(),
            };
            let text = format_named_character_values(vm, variable, table, target, format)?;
            self.presentation.append_print_text(text, false, true);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_ITEM" {
            let text = format_having_items(vm)?;
            self.presentation.append_print_text(text, false, true);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_PALAM" {
            let target = u64::try_from(integer_argument_value(&request.arguments, 0)?)
                .map_err(|_| RuntimeError::Internal("character index is negative".into()))?;
            let per_line = self
                .project_snapshot
                .as_ref()
                .map_or(3, |project| project.print_c_per_line.max(1));
            for (index, text) in format_character_palam(vm, target)?.into_iter().enumerate() {
                self.presentation
                    .append_column_cell(text, CellAlignment::Right);
                if (index + 1) % usize::try_from(per_line).unwrap_or(usize::MAX) == 0 {
                    self.presentation.flush_pending_line();
                }
            }
            self.presentation.flush_pending_line();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_SHOPITEM" {
            let project = self.project_snapshot.as_ref().ok_or_else(|| {
                RuntimeError::Internal("PRINT_SHOPITEM has no loaded project".into())
            })?;
            let per_line = project.print_c_per_line.max(1);
            let entries = format_shop_items(vm, project)?;
            for (index, (text, value)) in entries.into_iter().enumerate() {
                let token = self.allocate_interaction();
                self.presentation.append_button(
                    text,
                    era_runtime_protocol::ProtocolValue::Integer(value),
                    token,
                    Some(CellAlignment::Left),
                );
                self.command_intents.insert(token, VmValue::Integer(value));
                if (index + 1) % usize::try_from(per_line).unwrap_or(usize::MAX) == 0 {
                    self.presentation.flush_pending_line();
                }
            }
            self.presentation.flush_pending_line();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if is_print(&name) {
            let prepared =
                PreparedGenericPrint::prepare(&name, &request.arguments, self.force_kana_mode);
            let text = prepared.text;
            if name == "REUSELASTLINE" {
                self.presentation.print_temporary_line(text);
            } else if let Some(alignment) = column_print_alignment(&name) {
                // EmueraConsole.PrintC ignores empty strings entirely.
                if !text.is_empty() {
                    if print_uses_default_color(&name) {
                        self.presentation
                            .append_default_color_column_cell(text, alignment);
                    } else {
                        self.presentation.append_column_cell(text, alignment);
                    }
                    let values = self.presentation.last_column_auto_button_values();
                    let tokens = values
                        .iter()
                        .map(|_| self.allocate_interaction())
                        .collect::<Vec<_>>();
                    for (token, value) in self.presentation.bind_last_column_auto_buttons(&tokens) {
                        self.command_intents.insert(token, VmValue::Integer(value));
                    }
                }
            } else {
                let default_color = print_uses_default_color(&name);
                let plain = name.starts_with("PRINTPLAIN");
                let commit_at_end = print_commits_line(&name);
                if is_immediate_text_print(&name) && !text.contains('\n') {
                    PreparedGenericPrint { text }.apply_uncommitted(&mut self.presentation, &name);
                    commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
                    return self.emit_presentation();
                }
                let mut fragments = text.split('\n').peekable();
                while let Some(fragment) = fragments.next() {
                    let line_break = fragments.peek().is_some();
                    if default_color {
                        self.presentation.append_default_color_text(
                            fragment.to_owned(),
                            false,
                            false,
                        );
                    } else if plain {
                        self.presentation.append_plain_print_text(
                            fragment.to_owned(),
                            false,
                            false,
                        );
                    } else {
                        self.presentation
                            .append_print_text(fragment.to_owned(), false, false);
                    }
                    if line_break || commit_at_end {
                        if default_color {
                            self.presentation.force_default_color_new_line();
                        } else {
                            self.presentation.force_new_line();
                        }
                        if !plain {
                            bind_last_output_buttons(self);
                        }
                    }
                }
            }
            if name.ends_with('W') {
                let wait = InputWait {
                    wait_id: self.allocate_wait(),
                    kind: WaitKind::EnterKey,
                    stability: WaitStability::StableInput,
                    one_input: false,
                    stop_message_skip: false,
                    system_input: false,
                    mouse_input: false,
                    default_value: None,
                    deadline_ns: None,
                    display_time: false,
                    timeout_message: None,
                    submission_token: self.allocate_interaction(),
                    countdown_remaining_ms: None,
                };
                let pending = PendingInput {
                    host_request: Some(request.id),
                    wait,
                    result_name: None,
                    choices: BTreeMap::new(),
                    timeout_duration_ns: None,
                    post_input: None,
                };
                commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Pending {
                        stability: HostWaitStability::StableInput,
                        rebind_payload: encode_canonical(&pending.wait)?,
                    },
                )?;
                return self.open_wait(pending, false);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "UPDATECHECK" {
            let game_base = &vm.vm().artifact().project_data.static_data.game_base;
            if game_base.update_url.is_empty() {
                return commit_host_result_write(vm, request.id, 3);
            }
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::UpdateCheck {
                    request: request.id,
                },
                ServiceKind::Network,
                UPDATE_CHECK_OPERATION,
                UPDATE_CHECK_OPERATION_VERSION,
                &UpdateCheckRequest {
                    url: game_base.update_url.clone(),
                },
            );
        }
        if matches!(name.as_str(), "MOUSEX" | "MOUSEY" | "MOUSEB") {
            let coordinate = match name.as_str() {
                "MOUSEX" => PointerCoordinate::X,
                "MOUSEY" => PointerCoordinate::Y,
                _ => PointerCoordinate::Button,
            };
            let presentation_revision = self.presentation.revision();
            let environment_revision = self.projection_environment_revision;
            let projection_space_revision = self.projection_space_revision;
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::PointerState {
                    request: request.id,
                    coordinate,
                    presentation_revision,
                    environment_revision,
                    projection_space_revision,
                },
                ServiceKind::InputState,
                POINTER_STATE_OPERATION,
                POINTER_STATE_OPERATION_VERSION,
                &PointerStateRequest {
                    presentation_revision,
                    environment_revision,
                    projection_space_revision,
                },
            )
        } else if matches!(name.as_str(), "GETKEY" | "GETKEYTRIGGERED") {
            let key = match request.arguments.first() {
                Some(VmValue::Integer(value)) => match u8::try_from(*value) {
                    Ok(value) => value,
                    Err(_) => {
                        return commit_completion(
                            vm,
                            request.id,
                            VmHostCompletion::Ready(HostReady {
                                value: Some(VmValue::Integer(0)),
                                writes: Vec::new(),
                            }),
                        );
                    }
                },
                _ => {
                    return commit_completion(
                        vm,
                        request.id,
                        VmHostCompletion::Ready(HostReady {
                            value: Some(VmValue::Integer(0)),
                            writes: Vec::new(),
                        }),
                    );
                }
            };
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::GetKey {
                    request: request.id,
                    key_code: key,
                    triggered: name == "GETKEYTRIGGERED",
                },
                ServiceKind::InputState,
                GET_KEY_STATE_OPERATION,
                GET_KEY_STATE_OPERATION_VERSION,
                &GetKeyStateRequest { key_code: key },
            )
        } else if matches!(
            name.as_str(),
            "GETTIME" | "GETTIMES" | "GETMILLISECOND" | "GETSECOND"
        ) {
            let operation = match name.as_str() {
                "GETTIMES" => ClockOperation::Times,
                "GETMILLISECOND" => ClockOperation::Millisecond,
                "GETSECOND" => ClockOperation::Second,
                _ => ClockOperation::Time,
            };
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::LocalDateTime {
                    request: request.id,
                    operation,
                    result: request.import.import.result,
                },
                ServiceKind::Clock,
                LOCAL_DATE_TIME_OPERATION,
                LOCAL_DATE_TIME_OPERATION_VERSION,
                &LocalDateTimeRequest {},
            )
        } else {
            self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!("unsupported host import: {}", request.import.import.name),
                Some(request.origin.clone()),
            )
        }
    }
}

#[cfg(test)]
mod immediate_tests {
    use super::{
        ImmediateTagSplitTargets, ImmediateTextFormatting, RuntimeQueryEvaluation,
        RuntimeQueryState, evaluate_runtime_query, immediate_html_tag_split, immediate_text_value,
        is_immediate_committed_text_print, is_immediate_text_print,
        skips_runtime_command_immediately,
    };
    use crate::presentation::PresentationModel;
    use erabasic_vm::{PlaceDescriptor, VmValue};

    fn tag_split_targets(capacity: usize) -> ImmediateTagSplitTargets {
        let place = |index| PlaceDescriptor {
            indices: vec![index],
            ..PlaceDescriptor::default()
        };
        ImmediateTagSplitTargets {
            result: place(0),
            results: place(0),
            results_capacity: capacity,
        }
    }

    #[test]
    fn immediate_tag_split_preserves_default_target_write_semantics() {
        let targets = tag_split_targets(2);
        let ready =
            immediate_html_tag_split(&[VmValue::String("a<b>x</b>".into())], Some(&targets))
                .unwrap();
        assert_eq!(ready.writes.len(), 3);
        assert_eq!(ready.writes[0].value, VmValue::String("a".into()));
        assert_eq!(ready.writes[1].value, VmValue::String("<b>".into()));
        assert_eq!(ready.writes[2].value, VmValue::Integer(4));

        let empty = immediate_html_tag_split(
            &[VmValue::String(String::new())],
            Some(&tag_split_targets(2)),
        )
        .unwrap();
        assert_eq!(empty.writes.len(), 1);
        assert_eq!(empty.writes[0].value, VmValue::Integer(0));

        let malformed = immediate_html_tag_split(
            &[VmValue::String("a<b".into())],
            Some(&tag_split_targets(2)),
        )
        .unwrap();
        assert_eq!(malformed.writes.len(), 1);
        assert_eq!(malformed.writes[0].value, VmValue::Integer(-1));
    }

    #[test]
    fn immediate_tag_split_rejects_nondefault_or_mistyped_calls() {
        let targets = tag_split_targets(2);
        assert!(immediate_html_tag_split(&[VmValue::Integer(1)], Some(&targets)).is_none());
        assert!(
            immediate_html_tag_split(
                &[
                    VmValue::String("a".into()),
                    VmValue::StringPlace(Box::default()),
                ],
                Some(&targets),
            )
            .is_none()
        );
        assert!(
            immediate_html_tag_split(
                &[
                    VmValue::String("a".into()),
                    VmValue::StringPlace(Box::default()),
                    VmValue::IntegerPlace(Box::default()),
                ],
                Some(&targets),
            )
            .is_none()
        );
        assert!(immediate_html_tag_split(&[VmValue::String("a".into())], None).is_none());
    }

    #[test]
    fn layout_queries_are_never_classified_as_immediate_prints() {
        assert!(!is_immediate_text_print("PRINTCPERLINE"));
        assert!(!is_immediate_text_print("PRINTCLENGTH"));
        assert!(is_immediate_committed_text_print("PRINTL"));
        assert!(is_immediate_committed_text_print("PRINTFORMKL"));
        assert!(!is_immediate_committed_text_print("PRINTW"));
        assert!(!is_immediate_committed_text_print("PRINTFORMC"));
    }

    #[test]
    fn skipped_runtime_commands_use_the_immediate_lane_without_hiding_input_errors() {
        for name in ["PRINTL", "HTML_PRINT", "DRAWLINE", "WAITANYKEY", "INPUT"] {
            assert!(skips_runtime_command_immediately(
                "rustyera.text",
                name,
                true,
                false,
            ));
        }
        assert!(!skips_runtime_command_immediately(
            "rustyera.text",
            "INPUT",
            true,
            true,
        ));
        assert!(!skips_runtime_command_immediately(
            "rustyera.extension",
            "PRINTL",
            true,
            false,
        ));
        assert!(!skips_runtime_command_immediately(
            "rustyera.text",
            "GETCOLOR",
            true,
            false,
        ));
        assert!(!skips_runtime_command_immediately(
            "rustyera.text",
            "PRINTL",
            false,
            false,
        ));
    }

    #[test]
    fn pure_text_values_only_use_the_immediate_lane_when_the_slow_path_would_succeed() {
        let formatting = Some(ImmediateTextFormatting {
            bar_char_1: '*',
            bar_char_2: '.',
            money_first: true,
            money_label: "$",
        });
        assert_eq!(
            immediate_text_value(
                "TOSTR",
                &[VmValue::Integer(-12), VmValue::String("+#0;-#0".into())],
                formatting,
            ),
            Some(VmValue::String("-12".into()))
        );
        assert_eq!(
            immediate_text_value(
                "BARSTR",
                &[
                    VmValue::Integer(1),
                    VmValue::Integer(2),
                    VmValue::Integer(3),
                ],
                formatting,
            ),
            Some(VmValue::String("[*..]".into()))
        );
        assert_eq!(
            immediate_text_value(
                "MONEYSTR",
                &[VmValue::Integer(7), VmValue::String("0".into())],
                formatting,
            ),
            Some(VmValue::String("$7".into()))
        );
        assert!(
            immediate_text_value(
                "TOSTR",
                &[VmValue::Integer(1), VmValue::String("invalid[".into())],
                formatting,
            )
            .is_none()
        );
        assert!(
            immediate_text_value(
                "BARSTR",
                &[
                    VmValue::Integer(1),
                    VmValue::Integer(0),
                    VmValue::Integer(3),
                ],
                formatting,
            )
            .is_none()
        );
        assert!(
            immediate_text_value(
                "BARSTR",
                &[
                    VmValue::Integer(1),
                    VmValue::Integer(1),
                    VmValue::Integer(100),
                ],
                formatting,
            )
            .is_none()
        );
        assert!(
            immediate_text_value("TOSTR", &[VmValue::String("1".into())], formatting).is_none()
        );
        assert!(immediate_text_value("MONEYSTR", &[VmValue::Integer(7)], None).is_none());
    }

    #[test]
    fn runtime_query_evaluator_covers_every_immediate_query() {
        let presentation = PresentationModel::default();
        let state = RuntimeQueryState {
            skip_print: false,
            message_skip: true,
        };
        let cases = [
            (
                "HTML_ESCAPE",
                vec![VmValue::String("<&".into())],
                VmValue::String("&lt;&amp;".into()),
            ),
            (
                "HTML_TOPLAINTEXT",
                vec![VmValue::String("a&nbsp;b".into())],
                VmValue::String("a b".into()),
            ),
            ("CURRENTALIGN", vec![], VmValue::String("LEFT".into())),
            ("GETFONT", vec![], VmValue::String(presentation.font())),
            (
                "CURRENTREDRAW",
                vec![],
                VmValue::Integer(i64::from(presentation.redraw_enabled())),
            ),
            (
                "GETBGCOLOR",
                vec![],
                VmValue::Integer(presentation.background_rgb()),
            ),
            (
                "GETCOLOR",
                vec![],
                VmValue::Integer(presentation.foreground_rgb()),
            ),
            (
                "GETDEFBGCOLOR",
                vec![],
                VmValue::Integer(presentation.default_background_rgb()),
            ),
            (
                "GETDEFCOLOR",
                vec![],
                VmValue::Integer(presentation.default_foreground_rgb()),
            ),
            (
                "GETFOCUSCOLOR",
                vec![],
                VmValue::Integer(presentation.focus_rgb()),
            ),
            (
                "GETSTYLE",
                vec![],
                VmValue::Integer(presentation.style_bits()),
            ),
            ("ISSKIP", vec![], VmValue::Integer(0)),
            ("MESSKIP", vec![], VmValue::Integer(1)),
            ("MOUSESKIP", vec![], VmValue::Integer(1)),
            (
                "LINEISEMPTY",
                vec![],
                VmValue::Integer(i64::from(presentation.last_line_is_empty())),
            ),
        ];
        for (name, arguments, expected) in cases {
            assert_eq!(
                evaluate_runtime_query(name, &arguments, &presentation, state).unwrap(),
                RuntimeQueryEvaluation::Ready(expected),
                "{name}"
            );
        }
        assert_eq!(
            evaluate_runtime_query(
                "HTML_TOPLAINTEXT",
                &[VmValue::String("&#xD800;".into())],
                &presentation,
                state,
            )
            .unwrap(),
            RuntimeQueryEvaluation::MalformedHtml
        );
        assert_eq!(
            evaluate_runtime_query("UNKNOWN", &[], &presentation, state).unwrap(),
            RuntimeQueryEvaluation::Unhandled
        );
        assert!(
            evaluate_runtime_query(
                "HTML_TOPLAINTEXT",
                &[VmValue::Integer(1)],
                &presentation,
                state,
            )
            .is_err()
        );
    }

    #[test]
    fn runtime_query_evaluator_classifies_negative_printed_html_indexes() {
        assert_eq!(
            evaluate_runtime_query(
                "HTML_GETPRINTEDSTR",
                &[VmValue::Integer(-1)],
                &PresentationModel::default(),
                RuntimeQueryState {
                    skip_print: false,
                    message_skip: false,
                },
            )
            .unwrap(),
            RuntimeQueryEvaluation::InvalidPrintedHtmlIndex
        );
    }
}
