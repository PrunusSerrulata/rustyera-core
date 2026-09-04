#[derive(Clone, Copy, Eq, PartialEq)]
enum HostDispatchStatus {
    Unhandled,
    Handled,
}

#[derive(Clone, Copy)]
struct RuntimeQueryState {
    skip_print: bool,
    message_skip: bool,
    snake_display_state: bool,
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

struct PreparedHtmlColumnPrint {
    document: erabasic_html::HtmlDocument,
    warnings: Vec<erabasic_html::HtmlWarning>,
    alignment: CellAlignment,
    requested_pixels: i64,
    empty: bool,
}

fn document_has_unresolved_color_matrix(document: &erabasic_html::HtmlDocument) -> bool {
    fn node_has_unresolved_color_matrix(node: &erabasic_html::HtmlNode) -> bool {
        match node {
            erabasic_html::HtmlNode::Text { .. } => false,
            erabasic_html::HtmlNode::Element {
                semantic, children, ..
            } => {
                matches!(
                    semantic,
                    erabasic_html::HtmlElementSemantic::Image {
                        color_matrix: Some(erabasic_html::HtmlColorMatrix::Variable { .. }),
                        ..
                    }
                ) || children.iter().any(node_has_unresolved_color_matrix)
            }
        }
    }

    document.nodes.iter().any(node_has_unresolved_color_matrix)
}

fn resolve_document_color_matrices(
    vm: &RuntimeVm,
    fiber: erabasic_vm::FiberId,
    document: &mut erabasic_html::HtmlDocument,
) {
    fn resolve_node(
        vm: &RuntimeVm,
        fiber: erabasic_vm::FiberId,
        node: &mut erabasic_html::HtmlNode,
    ) {
        let erabasic_html::HtmlNode::Element {
            semantic, children, ..
        } = node
        else {
            return;
        };
        if let erabasic_html::HtmlElementSemantic::Image { color_matrix, .. } = semantic
            && let Some(erabasic_html::HtmlColorMatrix::Variable { name, indices }) = color_matrix
        {
            *color_matrix = read_named_color_matrix(vm, fiber, name, *indices)
                .map(erabasic_html::HtmlColorMatrix::Fixed);
        }
        for child in children {
            resolve_node(vm, fiber, child);
        }
    }

    for node in &mut document.nodes {
        resolve_node(vm, fiber, node);
    }
}

impl PreparedHtmlColumnPrint {
    fn prepare(name: &str, arguments: &[VmValue]) -> Result<Self, erabasic_html::HtmlError> {
        let markup = arguments.first().map_or_else(String::new, display_value);
        let (document, warnings) = erabasic_html::parse_document_with_warnings(&markup)?;
        Ok(Self {
            document,
            warnings,
            alignment: if name == "HTML_PRINTC" {
                CellAlignment::Right
            } else {
                CellAlignment::Left
            },
            requested_pixels: arguments.get(1).map_or(0, integer_value_or_zero),
            empty: markup.is_empty(),
        })
    }

    fn apply(self, presentation: &mut PresentationModel) -> bool {
        if self.empty {
            return false;
        }
        presentation.append_html_column_cell(self.document, self.alignment, self.requested_pixels);
        true
    }
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
        next_interaction_token(self.epoch, self.next_interaction_id)
    }
}

fn evaluate_runtime_query(
    name: &str,
    arguments: &(impl HostArgumentValues + ?Sized),
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
            let index = match arguments.argument(0) {
                Some(VmValue::Integer(value)) => *value,
                Some(_) | None => 0,
            };
            VmValue::String(presentation.display_line(index, state.snake_display_state))
        }
        "HTML_GETPRINTEDSTR" => {
            let raw_index = match arguments.argument(0) {
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
