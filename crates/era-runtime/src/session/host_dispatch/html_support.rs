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
                context: None,
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

fn emit_html_profile_error(
    session: &mut RuntimeSession,
    command: &str,
    error: &erabasic_html::HtmlError,
    origin: &erabasic_vm::VmExecutionOrigin,
    identity: &erabasic_compat::CompatibilityIdentity,
) -> Result<(), RuntimeError> {
    let attribute_error = matches!(
        error.kind,
        erabasic_html::HtmlErrorKind::InvalidAttribute
            | erabasic_html::HtmlErrorKind::DuplicateAttribute
            | erabasic_html::HtmlErrorKind::InvalidAttributeValue
    );
    session.emit(
        RuntimeMessage::Diagnostic(ProtocolDiagnostic {
            context: Some(Box::new(
                era_runtime_protocol::CompatibilityDiagnosticContext {
                    identity: Some(identity.clone()),
                    artifact: None,
                    project_load_id: None,
                    runtime_epoch: Some(session.epoch.0),
                    generation: Some(origin.generation.0),
                    stage: "runtime.html".into(),
                    api: Some(command.into()),
                    required_capability: None,
                },
            )),
            code: if attribute_error {
                "runtime.html.profile_attribute_rejected"
            } else {
                "runtime.html.profile_markup_rejected"
            }
            .into(),
            level: RuntimeLogLevel::Error,
            message: format!(
                "{command} rejected {:?} for profile {} at UTF-8 bytes {}..{}",
                error.kind, identity.profile, error.start, error.end
            ),
            source: protocol_execution_origin(origin.clone()).source,
            notification: DiagnosticNotification::default(),
        }),
        None,
    )
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
