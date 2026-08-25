//! Translation of VM host requests into runtime-owned semantic operations.

// This is one part of the same split `RuntimeSession` implementation.
#[allow(clippy::wildcard_imports)]
use super::*;

mod control;
mod graphics;
mod presentation;
mod services;
mod storage;

pub(in crate::session) use services::immediate_tag_split_targets;

#[derive(Clone, Copy, Eq, PartialEq)]
enum HostDispatchStatus {
    Unhandled,
    Handled,
}

#[derive(Clone, Copy)]
struct RuntimeQueryState {
    skip_print: bool,
    message_skip: bool,
}

#[derive(Debug, Eq, PartialEq)]
enum RuntimeQueryEvaluation {
    Ready(VmValue),
    MalformedHtml,
    InvalidPrintedHtmlIndex,
    Unhandled,
}

fn split_html_tags(source: &str) -> Result<Vec<String>, erabasic_html::Error> {
    erabasic_html::split_tags(source).map(|tokens| {
        tokens
            .into_iter()
            .map(|token| match token {
                erabasic_html::Token::Text(value) | erabasic_html::Token::Tag(value) => value,
            })
            .collect()
    })
}

struct PreparedHtmlPrint {
    document: erabasic_html::HtmlDocument,
    warnings: Vec<erabasic_html::HtmlWarning>,
    inline: bool,
}

impl PreparedHtmlPrint {
    fn prepare(arguments: &[VmValue]) -> Result<Self, erabasic_html::HtmlError> {
        let markup = arguments.first().map_or_else(String::new, display_value);
        let (document, warnings) = erabasic_html::parse_document_with_warnings(&markup)?;
        Ok(Self {
            document,
            warnings,
            inline: arguments.get(1).map_or(0, integer_value_or_zero) != 0,
        })
    }

    fn apply(self, presentation: &mut PresentationModel) {
        if self.inline {
            presentation.append_html_inline(self.document);
        } else {
            presentation.append_html(self.document);
        }
    }
}

#[derive(Debug)]
enum PresentationStatePreparationError {
    Alignment,
    FontStyle(RuntimeError),
    Color(&'static str),
}

enum PreparedPresentationState {
    Alignment(LineAlignment),
    FontStyle(i64),
    Bold,
    Italic,
    Regular,
    Font(Option<String>),
    Foreground(i64),
    ResetForeground,
}

impl PreparedPresentationState {
    fn prepare(
        name: &str,
        arguments: &[VmValue],
    ) -> Result<Option<Self>, PresentationStatePreparationError> {
        let prepared = match name {
            "ALIGNMENT" => {
                let alignment = match arguments.first() {
                    Some(VmValue::String(value)) if value.eq_ignore_ascii_case("CENTER") => {
                        LineAlignment::Center
                    }
                    Some(VmValue::String(value)) if value.eq_ignore_ascii_case("RIGHT") => {
                        LineAlignment::Right
                    }
                    Some(VmValue::String(value)) if value.eq_ignore_ascii_case("LEFT") => {
                        LineAlignment::Left
                    }
                    _ => return Err(PresentationStatePreparationError::Alignment),
                };
                Self::Alignment(alignment)
            }
            "FONTSTYLE" => Self::FontStyle(
                integer_argument_value(arguments, 0)
                    .map_err(PresentationStatePreparationError::FontStyle)?,
            ),
            "FONTBOLD" => Self::Bold,
            "FONTITALIC" => Self::Italic,
            "FONTREGULAR" => Self::Regular,
            "SETFONT" => Self::Font(arguments.first().map(display_value)),
            "SETCOLOR" => Self::Foreground(
                color_argument_value(arguments)
                    .map_err(PresentationStatePreparationError::Color)?,
            ),
            "RESETCOLOR" => Self::ResetForeground,
            _ => return Ok(None),
        };
        Ok(Some(prepared))
    }

    fn apply(self, presentation: &mut PresentationModel) {
        match self {
            Self::Alignment(alignment) => presentation.set_alignment(alignment),
            Self::FontStyle(bits) => presentation.set_font_style(bits),
            Self::Bold => presentation.set_bold(true),
            Self::Italic => presentation.set_italic(true),
            Self::Regular => presentation.clear_font_style(),
            Self::Font(family) => presentation.set_font(family),
            Self::Foreground(color) => presentation.set_foreground(color),
            Self::ResetForeground => presentation.reset_foreground(),
        }
    }
}

enum PreparedLineEdit {
    AppendSeparator(String),
    Clear(usize),
}

impl PreparedLineEdit {
    fn prepare(name: &str, arguments: &[VmValue]) -> Option<Self> {
        if matches!(name, "DRAWLINE" | "CUSTOMDRAWLINE" | "DRAWLINEFORM") {
            return Some(Self::AppendSeparator(
                arguments.first().map_or_else(|| "-".into(), display_value),
            ));
        }
        if name == "CLEARLINE" {
            let count = arguments
                .first()
                .and_then(|value| match value {
                    VmValue::Integer(value) => usize::try_from(*value).ok(),
                    _ => None,
                })
                .unwrap_or(1);
            return Some(Self::Clear(count));
        }
        None
    }

    fn apply(self, presentation: &mut PresentationModel) {
        match self {
            Self::AppendSeparator(pattern) => presentation.append_separator(pattern),
            Self::Clear(count) => presentation.delete_last_lines(count),
        }
    }
}

struct HtmlInteractionBindings<'a> {
    epoch: u64,
    next_interaction_id: &'a mut u64,
    button_generation: u64,
    command_intents: &'a mut BTreeMap<InteractionToken, VmValue>,
}

impl HtmlInteractionBindings<'_> {
    fn allocate_interaction(&mut self) -> InteractionToken {
        let token = InteractionToken {
            epoch: self.epoch,
            id: *self.next_interaction_id,
        };
        *self.next_interaction_id = (*self.next_interaction_id).saturating_add(1);
        token
    }
}

fn evaluate_runtime_query(
    name: &str,
    arguments: &[VmValue],
    presentation: &PresentationModel,
    state: RuntimeQueryState,
) -> Result<RuntimeQueryEvaluation, RuntimeError> {
    let value = match name {
        "HTML_ESCAPE" | "HTML_TOPLAINTEXT" => {
            let source = string_argument_value(arguments, 0, name)?;
            let value = if name == "HTML_ESCAPE" {
                erabasic_html::escape(source)
            } else {
                let Ok(value) = erabasic_html::to_plain_text(source) else {
                    return Ok(RuntimeQueryEvaluation::MalformedHtml);
                };
                value
            };
            VmValue::String(value)
        }
        "CURRENTALIGN" | "GETFONT" => VmValue::String(if name == "GETFONT" {
            presentation.font()
        } else {
            match presentation.alignment() {
                LineAlignment::Left => "LEFT",
                LineAlignment::Center => "CENTER",
                LineAlignment::Right => "RIGHT",
            }
            .into()
        }),
        "CURRENTREDRAW" => VmValue::Integer(i64::from(presentation.redraw_enabled())),
        "GETBGCOLOR" => VmValue::Integer(presentation.background_rgb()),
        "GETCOLOR" => VmValue::Integer(presentation.foreground_rgb()),
        "GETDEFBGCOLOR" => VmValue::Integer(presentation.default_background_rgb()),
        "GETDEFCOLOR" => VmValue::Integer(presentation.default_foreground_rgb()),
        "GETFOCUSCOLOR" => VmValue::Integer(presentation.focus_rgb()),
        "GETSTYLE" => VmValue::Integer(presentation.style_bits()),
        "GETDISPLAYLINE" => {
            let index = match arguments.first() {
                Some(VmValue::Integer(value)) => usize::try_from(*value).ok(),
                Some(_) | None => Some(0),
            };
            VmValue::String(
                index.map_or_else(String::new, |index| presentation.display_line(index)),
            )
        }
        "HTML_GETPRINTEDSTR" => {
            let raw_index = match arguments.first() {
                Some(VmValue::Integer(value)) => *value,
                Some(_) | None => 0,
            };
            if raw_index < 0 {
                return Ok(RuntimeQueryEvaluation::InvalidPrintedHtmlIndex);
            }
            VmValue::String(
                usize::try_from(raw_index)
                    .ok()
                    .map_or_else(String::new, |index| presentation.printed_html_line(index)),
            )
        }
        "ISSKIP" => VmValue::Integer(i64::from(state.skip_print)),
        "MESSKIP" | "MOUSESKIP" => VmValue::Integer(i64::from(state.message_skip)),
        "LINEISEMPTY" => VmValue::Integer(i64::from(presentation.last_line_is_empty())),
        _ => return Ok(RuntimeQueryEvaluation::Unhandled),
    };
    Ok(RuntimeQueryEvaluation::Ready(value))
}

impl RuntimeSession {
    #[allow(clippy::single_match_else, clippy::too_many_lines)]
    pub(super) fn handle_host_call(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        if let Some(time) = self.candidate_clock {
            match request.import.contract.candidate {
                erabasic_bytecode::CandidatePolicy::Forbidden => {
                    return Err(RuntimeError::Internal(format!(
                        "{} is forbidden during candidate SAVEINFO execution",
                        request.import.import.name
                    )));
                }
                erabasic_bytecode::CandidatePolicy::FrozenClock => {
                    return complete_frozen_clock(vm, request, time);
                }
                erabasic_bytecode::CandidatePolicy::ReadOnly
                | erabasic_bytecode::CandidatePolicy::CloneCommit
                | erabasic_bytecode::CandidatePolicy::BufferedEffect => {}
            }
        }
        if request
            .import
            .import
            .namespace
            .eq_ignore_ascii_case("rustyera.extension")
        {
            return self.issue_extension(vm, request);
        }
        let name = request.import.import.name.to_ascii_uppercase();
        if name == "SKIPDISP" {
            self.skip_print = integer_argument_value(&request.arguments, 0)? != 0;
            self.user_defined_skip = self.skip_print;
            // Host calls execute while the caller-pumped drive loop temporarily
            // owns the VM, so RESULT must be resolved through that VM rather than
            // through the session's temporarily empty VM slot.
            return commit_host_result_write(vm, request.id, i64::from(self.skip_print));
        }
        if name == "SKIPLOG" {
            self.message_skip = integer_argument_value(&request.arguments, 0)? != 0;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "NOSKIP" {
            self.saved_skip = self.skip_print;
            self.skip_print = false;
            return commit_integer_result(vm, request.id, 1);
        }
        if name == "ENDNOSKIP" {
            if self.saved_skip {
                self.skip_print = true;
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        match evaluate_runtime_query(
            &name,
            &request.arguments,
            &self.presentation,
            RuntimeQueryState {
                skip_print: self.skip_print,
                message_skip: self.message_skip,
            },
        )? {
            RuntimeQueryEvaluation::Ready(value) => {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: Some(value),
                        writes: Vec::new(),
                    }),
                );
            }
            RuntimeQueryEvaluation::MalformedHtml => {
                return self.fault(
                    FaultCode::VmFault,
                    "malformed HTML text",
                    Some(request.origin.clone()),
                );
            }
            RuntimeQueryEvaluation::InvalidPrintedHtmlIndex => {
                return self.fault(
                    FaultCode::VmFault,
                    "HTML_GETPRINTEDSTR line number must be non-negative",
                    Some(request.origin.clone()),
                );
            }
            RuntimeQueryEvaluation::Unhandled => {}
        }
        if name == "ASSERT" {
            if integer_argument_value(&request.arguments, 0)? == 0 {
                return self.fault(
                    FaultCode::VmFault,
                    "ASSERT failed",
                    Some(request.origin.clone()),
                );
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "THROW" {
            let message = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            return self.fault(FaultCode::VmFault, &message, Some(request.origin.clone()));
        }
        if name == "FORCEKANA" {
            let mode = integer_argument_value(&request.arguments, 0)?;
            let Ok(mode) = u8::try_from(mode) else {
                return self.fault(
                    FaultCode::VmFault,
                    "FORCEKANA mode must be between 0 and 3",
                    Some(request.origin.clone()),
                );
            };
            if mode > 3 {
                return self.fault(
                    FaultCode::VmFault,
                    "FORCEKANA mode must be between 0 and 3",
                    Some(request.origin.clone()),
                );
            }
            self.force_kana_mode = mode;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if matches!(name.as_str(), "UPCHECK" | "CUPCHECK") {
            let (character, character_scoped) = if name == "CUPCHECK" {
                let character = u64::try_from(integer_argument_value(&request.arguments, 0)?)
                    .map_err(|_| RuntimeError::Internal("character index is negative".into()))?;
                (character, true)
            } else {
                let target = read_runtime_integer(vm, "TARGET", &[], None)?;
                let Ok(character) = u64::try_from(target) else {
                    clear_upcheck_arrays(vm, false, None)?;
                    return commit_completion(
                        vm,
                        request.id,
                        VmHostCompletion::Ready(HostReady::empty()),
                    );
                };
                (character, false)
            };
            let lines = apply_upcheck(vm, character, character_scoped)?;
            if !self.skip_print {
                for line in lines {
                    self.presentation.append_print_text(line, false, true);
                }
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "ISACTIVE" {
            let value = self.client_focused;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(value))),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "SETANIMETIMER" {
            let milliseconds = integer_argument_value(&request.arguments, 0)?;
            self.project_snapshot
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("SETANIMETIMER has no project".into()))?
                .resource_graph
                .set_animation_timer(milliseconds);
            self.sync_resource_replay();
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_presentation();
        }
        if self.controller.step == SystemStep::TrainEventComEnd
            && matches!(name.as_str(), "WAIT" | "WAITANYKEY" | "FORCEWAIT" | "TWAIT")
        {
            self.controller.event_com_end_wait_required = false;
        }
        if self.skip_print && is_runtime_print_command(&name) {
            if self.user_defined_skip && is_input_command(&name) {
                return self.fault(
                    FaultCode::VmFault,
                    "an input command cannot execute while user SKIPDISP is active; wrap it in NOSKIP/ENDNOSKIP",
                    Some(request.origin.clone()),
                );
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        let mut status = HostDispatchStatus::Unhandled;
        let result = self.dispatch_control(vm, request, &name, &mut status);
        if status == HostDispatchStatus::Handled {
            return result;
        }
        let result = self.dispatch_storage(vm, request, &name, &mut status);
        if status == HostDispatchStatus::Handled {
            return result;
        }
        let result = self.dispatch_presentation(vm, request, &name, &mut status);
        if status == HostDispatchStatus::Handled {
            return result;
        }
        let result = self.dispatch_graphics(vm, request, &name, &mut status);
        if status == HostDispatchStatus::Handled {
            return result;
        }
        self.dispatch_services(vm, request, &name)
    }

    // The typed operation tuple is deliberately explicit at this single protocol edge.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn issue_host_service<T: minicbor::Encode<()>>(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        completion: ExternalCompletion,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
        payload: &T,
    ) -> Result<(), RuntimeError> {
        if self.service_capabilities.get(&(kind, operation.to_owned())) != Some(&operation_version)
        {
            return self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!(
                    "frontend did not negotiate service {kind:?}/{operation} {operation_version:?}"
                ),
                Some(request.origin.clone()),
            );
        }
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        let request_id = self.allocate_request()?;
        self.operations
            .insert_service(request_id, PendingService::Host(completion));
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
                deadline_ns: None,
            }),
            None,
        )
    }

    fn projection_query_context(&self) -> ProjectionQueryContext {
        ProjectionQueryContext {
            presentation_revision: self.presentation.revision(),
            environment_revision: self.projection_environment_revision,
            projection_space_revision: self.projection_space_revision,
        }
    }

    fn presentation_observation_context(&mut self) -> Result<ProjectionQueryContext, RuntimeError> {
        // A presentation query is an observation barrier: its frontend response must describe
        // the canonical state that existed when the request was issued, even while ordinary
        // animation frames are being collapsed during a continuous message skip.
        self.flush_presentation_for_observation()?;
        Ok(self.projection_query_context())
    }

    fn issue_extension(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let operation = request.import.import.name.to_ascii_lowercase();
        let declaration = self
            .project_snapshot
            .as_ref()
            .and_then(|project| project.extensions.get(&operation))
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Internal(format!("extension import {operation} has no declaration"))
            })?;
        let mut arguments = Vec::with_capacity(request.arguments.len());
        let mut mutable_places = Vec::with_capacity(request.arguments.len());
        for (ordinal, argument) in request.arguments.iter().enumerate() {
            let (value, place) = match argument {
                VmValue::Integer(value) => {
                    (era_runtime_protocol::ProtocolValue::Integer(*value), None)
                }
                VmValue::String(value) => (
                    era_runtime_protocol::ProtocolValue::String(value.clone()),
                    None,
                ),
                VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => {
                    let value = vm
                        .read_host_place(request.fiber, place)
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                    let value = match value {
                        VmValue::Integer(value) => {
                            era_runtime_protocol::ProtocolValue::Integer(value)
                        }
                        VmValue::String(value) => {
                            era_runtime_protocol::ProtocolValue::String(value)
                        }
                        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                            return Err(RuntimeError::Internal(
                                "reading an extension place returned another place".into(),
                            ));
                        }
                    };
                    let declared_type = declaration
                        .arguments
                        .get(ordinal)
                        .map_or(era_runtime_protocol::ExtensionValueType::Any, |argument| {
                            argument.value_type
                        });
                    (value, Some((place.as_ref().clone(), declared_type)))
                }
            };
            arguments.push(value);
            mutable_places.push(place);
        }
        let invocation = era_runtime_protocol::ExtensionInvocation {
            extension_id: declaration.id,
            arguments,
        };
        self.issue_host_service(
            vm,
            request,
            ExternalCompletion::Extension {
                request: request.id,
                return_type: declaration.return_type,
                mutable_places,
            },
            ServiceKind::Extension,
            &declaration.operation,
            declaration.operation_version,
            &invocation,
        )
    }

    pub(super) fn issue_platform_effect<T: minicbor::Encode<()>>(
        &mut self,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
        payload: &T,
    ) -> Result<(), RuntimeError> {
        if self.service_capabilities.get(&(kind, operation.to_owned())) != Some(&operation_version)
        {
            return self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.platform_capability_unavailable".into(),
                    level: RuntimeLogLevel::Warning,
                    message: format!("frontend did not negotiate service {kind:?}/{operation}"),
                    source: None,
                    notification: DiagnosticNotification::default(),
                }),
                None,
            );
        }
        let request_id = self.allocate_request()?;
        self.operations.insert_service(
            request_id,
            PendingService::PlatformEffect {
                operation: operation.into(),
            },
        );
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
                deadline_ns: None,
            }),
            None,
        )
    }

    pub(super) fn issue_host_storage(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        pending: PendingStorage,
        namespace: StorageNamespace,
        operation: StorageOperation,
        relative_path: String,
    ) -> Result<(), RuntimeError> {
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        self.issue_storage(pending, namespace, operation, relative_path)
    }
}
fn bind_html_document(
    bindings: &mut HtmlInteractionBindings<'_>,
    document: &mut erabasic_html::HtmlDocument,
) {
    fn visit(
        bindings: &mut HtmlInteractionBindings<'_>,
        nodes: &mut [erabasic_html::HtmlNode],
        buttons_suppressed: bool,
    ) {
        for node in nodes {
            let erabasic_html::HtmlNode::Element {
                kind,
                attributes,
                children,
                interaction,
                ..
            } = node
            else {
                continue;
            };
            match kind {
                erabasic_html::HtmlElementKind::Button if !buttons_suppressed => {
                    let Some(value) = attributes
                        .iter()
                        .find(|attribute| attribute.name == "value")
                        .map(|attribute| attribute.value.clone())
                    else {
                        visit(bindings, children, buttons_suppressed);
                        continue;
                    };
                    let token = bindings.allocate_interaction();
                    let vm_value = value
                        .parse::<i64>()
                        .map_or_else(|_| VmValue::String(value.clone()), VmValue::Integer);
                    let (integer_value, string_value) = match &vm_value {
                        VmValue::Integer(value) => (Some(*value), None),
                        VmValue::String(value) => (None, Some(value.clone())),
                        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => unreachable!(),
                    };
                    *interaction = Some(erabasic_html::HtmlInteraction {
                        epoch: token.epoch,
                        id: token.id,
                        integer_value,
                        string_value,
                        generation: bindings.button_generation,
                        enabled: true,
                    });
                    bindings.command_intents.insert(token, vm_value);
                }
                erabasic_html::HtmlElementKind::ClearButton => {
                    // clearbutton suppresses buttonization only for its subtree;
                    // it never invalidates interactions already printed.
                    visit(bindings, children, true);
                    continue;
                }
                _ => {}
            }
            visit(bindings, children, buttons_suppressed);
        }
    }
    visit(bindings, &mut document.nodes, false);
}

fn emit_html_warnings(
    session: &mut RuntimeSession,
    command: &str,
    warnings: &[erabasic_html::HtmlWarning],
    origin: &erabasic_vm::VmExecutionOrigin,
) -> Result<(), RuntimeError> {
    let source = protocol_execution_origin(origin.clone()).source;
    for warning in warnings {
        let crossed = warning
            .crossed
            .iter()
            .map(|kind| format!("<{}>", kind.tag_name()))
            .collect::<Vec<_>>()
            .join(", ");
        let (code, message) = match warning.kind {
            erabasic_html::HtmlWarningKind::CrossedClosingTag => (
                "runtime.html.nonstandard_crossed_closing_tag",
                format!(
                    "{command} normalized non-standard crossed closing tag </{}> at UTF-8 bytes {}..{} across open {crossed}; use properly nested markup",
                    warning.closing.tag_name(),
                    warning.start,
                    warning.end
                ),
            ),
        };
        session.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: code.into(),
                level: RuntimeLogLevel::Warning,
                message,
                source: source.clone(),
                notification: DiagnosticNotification::LogOnly,
            }),
            None,
        )?;
    }
    Ok(())
}

fn bind_last_output_buttons(session: &mut RuntimeSession) {
    let count = session.presentation.last_line_auto_button_values().len();
    let tokens = (0..count)
        .map(|_| session.allocate_interaction())
        .collect::<Vec<_>>();
    for (token, value) in session.presentation.bind_last_line_auto_buttons(&tokens) {
        session
            .command_intents
            .insert(token, VmValue::Integer(value));
    }
}

fn mixed_lengths(arguments: &[VmValue]) -> Result<Vec<PresentationLength>, RuntimeError> {
    if !arguments.len().is_multiple_of(2) {
        return Err(RuntimeError::Internal(
            "mixed-number host arguments are not value/unit pairs".into(),
        ));
    }
    arguments
        .chunks_exact(2)
        .map(|pair| {
            let VmValue::Integer(value) = pair[0] else {
                return Err(RuntimeError::Internal(
                    "mixed-number value is not an integer".into(),
                ));
            };
            let VmValue::Integer(unit) = pair[1] else {
                return Err(RuntimeError::Internal(
                    "mixed-number unit is not an integer".into(),
                ));
            };
            let is_px = unit != 0;
            Ok(if is_px {
                PresentationLength::Logical(era_runtime_protocol::LogicalLength(
                    value.saturating_mul(1_000),
                ))
            } else {
                PresentationLength::FontHeightHundredths(value)
            })
        })
        .collect()
}

fn append_mixed_html_attribute(
    output: &mut String,
    name: &str,
    value: Option<&PresentationLength>,
    line_height: era_runtime_protocol::LogicalLength,
) {
    let Some(value) = value else {
        return;
    };
    let (number, suffix) = match value {
        PresentationLength::Logical(value) => (value.0 / 1_000, "px"),
        PresentationLength::FontHeightHundredths(value) => {
            (value.saturating_mul(line_height.0) / 100_000, "")
        }
    };
    if number != 0 {
        let _ = write!(output, " {name}='{number}{suffix}'");
    }
}
