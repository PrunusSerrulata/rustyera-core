use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions, CsvSourceLocation,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnabledLine {
    pub text: String,
    pub source: CsvSourceLocation,
}

/// Reproduces `EraStreamReader.ReadEnabledLine` over a submitted UTF-8 string.
#[allow(clippy::too_many_lines)]
pub(crate) fn enabled_lines(
    path: &str,
    content: &str,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> Vec<EnabledLine> {
    let physical = physical_lines(content);
    let mut result = Vec::new();
    let mut index = 0;
    while index < physical.len() {
        let (line, start, end) = physical[index];
        let current_line = line_number(index);
        let bom_bytes = if index == 0 && line.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        let decoded_line = &line[bom_bytes..];
        let (skipped, is_comment) = skip_enabled_prefix(
            decoded_line,
            options.allow_full_width_space,
            options.debug_mode,
        );
        let enabled = &decoded_line[skipped..];
        if enabled.is_empty() || is_comment {
            index += 1;
            continue;
        }
        if enabled.starts_with('}') {
            diagnostics.push(continuation_error(
                path,
                current_line,
                start + bom_bytes + skipped,
                end,
                "unexpected continuation terminator",
            ));
            break;
        }
        if enabled.starts_with('{') {
            if decoded_line.trim() != "{" {
                diagnostics.push(continuation_error(
                    path,
                    current_line,
                    start + bom_bytes + skipped,
                    end,
                    "characters follow the continuation opener",
                ));
                break;
            }
            let logical_start = current_line;
            let logical_byte_start = start;
            let mut joined = String::new();
            index += 1;
            let mut closed = false;
            let mut failed = false;
            let mut logical_end = end;
            while index < physical.len() {
                let (body, body_start, body_end) = physical[index];
                let trimmed = body.trim_start();
                logical_end = body_end;
                if trimmed.starts_with('}') {
                    if trimmed.trim_end() == "}" {
                        closed = true;
                    } else {
                        diagnostics.push(continuation_error(
                            path,
                            line_number(index),
                            body_start,
                            body_end,
                            "characters follow the continuation terminator",
                        ));
                        failed = true;
                    }
                    index += 1;
                    break;
                }
                if trimmed == "{" {
                    diagnostics.push(continuation_error(
                        path,
                        line_number(index),
                        body_start,
                        body_end,
                        "nested continuation opener",
                    ));
                    failed = true;
                    index += 1;
                    break;
                }
                joined.push_str(body);
                joined.push_str(&options.continuation_separator.replace('"', ""));
                index += 1;
            }
            if failed {
                break;
            }
            if !closed {
                if index >= physical.len() {
                    diagnostics.push(continuation_error(
                        path,
                        logical_start,
                        logical_byte_start,
                        logical_end,
                        "continuation is not closed",
                    ));
                }
                break;
            }
            let (skipped, is_comment) =
                skip_enabled_prefix(&joined, options.allow_full_width_space, options.debug_mode);
            let skipped = if is_comment { joined.len() } else { skipped };
            result.push(EnabledLine {
                text: joined[skipped..].to_owned(),
                source: CsvSourceLocation {
                    relative_path: path.to_owned(),
                    physical_line: logical_start,
                    logical_line: logical_start,
                    byte_start: logical_byte_start,
                    byte_end: logical_end,
                },
            });
            continue;
        }
        result.push(EnabledLine {
            text: enabled.to_owned(),
            source: CsvSourceLocation {
                relative_path: path.to_owned(),
                physical_line: current_line,
                logical_line: current_line,
                byte_start: start + bom_bytes + skipped,
                byte_end: end,
            },
        });
        index += 1;
    }
    result
}

fn continuation_error(
    path: &str,
    line: u32,
    start: usize,
    end: usize,
    message: &str,
) -> CsvDiagnostic {
    CsvDiagnostic::new(
        CsvDiagnosticCode::MalformedContinuation,
        CsvDiagnosticSeverity::Error,
        3,
        path,
        Some(CsvSourceLocation {
            relative_path: path.to_owned(),
            physical_line: line,
            logical_line: line,
            byte_start: start,
            byte_end: end,
        }),
        message,
    )
}

fn skip_leading(value: &str, allow_full_width_space: bool) -> usize {
    let mut bytes = 0;
    for character in value.chars() {
        if character == ' ' || character == '\t' || (allow_full_width_space && character == '　') {
            bytes += character.len_utf8();
        } else {
            break;
        }
    }
    bytes
}

fn skip_enabled_prefix(
    value: &str,
    allow_full_width_space: bool,
    debug_mode: bool,
) -> (usize, bool) {
    let mut offset = 0;
    loop {
        offset += skip_leading(&value[offset..], allow_full_width_space);
        let rest = &value[offset..];
        if rest.starts_with(";!;")
            || rest.starts_with(";^;")
            || (debug_mode && rest.starts_with(";#;"))
        {
            offset += 3;
            continue;
        }
        return (offset, rest.starts_with(';'));
    }
}

fn line_number(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

fn physical_lines(content: &str) -> Vec<(&str, usize, usize)> {
    let mut result = Vec::new();
    let mut offset = 0;
    for line_with_newline in content.split_inclusive('\n') {
        let without_newline = line_with_newline
            .strip_suffix('\n')
            .unwrap_or(line_with_newline);
        let line = without_newline
            .strip_suffix('\r')
            .unwrap_or(without_newline);
        result.push((line, offset, offset + line.len()));
        offset += line_with_newline.len();
    }
    result
}
