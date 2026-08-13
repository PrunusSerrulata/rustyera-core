use std::fmt::Write as _;

use era_runtime_protocol::{Color, DisplayRun, LogicalLength, PresentationLength, ProtocolValue};

pub(in crate::presentation) fn append_log_run(output: &mut String, run: &DisplayRun) {
    match run {
        DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => {
            output.push_str(text);
        }
        DisplayRun::Button { runs, .. } => {
            for run in runs {
                append_log_run(output, run);
            }
        }
        DisplayRun::HtmlDocument { document } => {
            output.push_str(&erabasic_html::serialize_document(document));
        }
        DisplayRun::Image { alt_text, .. } => {
            if let Some(text) = alt_text {
                output.push_str(text);
            }
        }
        DisplayRun::Shape { .. } | DisplayRun::Space { .. } => {}
        DisplayRun::ColumnCell { content, .. } => {
            for run in content {
                append_log_run(output, run);
            }
            output.push(' ');
        }
        DisplayRun::Separator { pattern, .. } => output.push_str(pattern),
    }
}

#[allow(clippy::too_many_lines)]
pub(in crate::presentation) fn append_html_run(
    output: &mut String,
    run: &DisplayRun,
    line_height: LogicalLength,
) {
    match run {
        DisplayRun::Text { text, style, .. } | DisplayRun::TextLayout { text, style, .. } => {
            let mut value = erabasic_html::escape(text);
            if style.strikeout {
                value = format!("<s>{value}</s>");
            }
            if style.underline {
                value = format!("<u>{value}</u>");
            }
            if style.italic {
                value = format!("<i>{value}</i>");
            }
            if style.bold {
                value = format!("<b>{value}</b>");
            }
            output.push_str(&value);
        }
        DisplayRun::Button {
            runs, value, title, ..
        } => {
            output.push_str("<button value='");
            let value = match value {
                ProtocolValue::Integer(value) => value.to_string(),
                ProtocolValue::String(value) => value.clone(),
                ProtocolValue::Boolean(value) => i64::from(*value).to_string(),
                ProtocolValue::Bytes(_) => String::new(),
            };
            output.push_str(&erabasic_html::escape(&value));
            if let Some(title) = title {
                output.push_str("' title='");
                output.push_str(&erabasic_html::escape(title));
            }
            output.push_str("'>");
            for run in runs {
                append_html_run(output, run, line_height);
            }
            output.push_str("</button>");
        }
        DisplayRun::HtmlDocument { document } => {
            output.push_str(&erabasic_html::serialize_document(document));
        }
        DisplayRun::Image { placement, .. } => {
            output.push_str("<img src='");
            output.push_str(&erabasic_html::escape(&placement.resource_id));
            if let Some(resource) = &placement.hover_resource_id {
                output.push_str("' srcb='");
                output.push_str(&erabasic_html::escape(resource));
            }
            if let Some(resource) = &placement.mask_resource_id {
                output.push_str("' srcm='");
                output.push_str(&erabasic_html::escape(resource));
            }
            for (name, value) in [
                ("height", placement.requested_height.as_ref()),
                ("width", placement.requested_width.as_ref()),
                ("ypos", placement.requested_y.as_ref()),
            ] {
                if let Some(value) = value {
                    output.push_str("' ");
                    output.push_str(name);
                    output.push_str("='");
                    append_presentation_length(output, value, line_height);
                }
            }
            output.push_str("'>");
        }
        DisplayRun::Shape { shape } => {
            output.push_str("<shape type='");
            output.push_str(&erabasic_html::escape(&shape.kind));
            output.push_str("' param='");
            for (index, value) in shape.parameters.iter().enumerate() {
                if index != 0 {
                    output.push_str(", ");
                }
                append_raw_mixed_length(output, value);
            }
            output.push('\'');
            if shape
                .foreground
                .is_some_and(|color| color != super::default_style().foreground)
            {
                output.push_str(" color='");
                append_html_color(output, shape.foreground.expect("checked foreground"));
                output.push('\'');
            }
            if let Some(background) = shape.background {
                output.push_str(" bcolor='");
                append_html_color(output, background);
                output.push('\'');
            }
            output.push('>');
        }
        DisplayRun::ColumnCell { content, .. } => {
            for run in content {
                append_html_run(output, run, line_height);
            }
        }
        DisplayRun::Separator { pattern, .. } => {
            output.push_str(&erabasic_html::escape(pattern));
        }
        DisplayRun::Space { width } => {
            output.push_str("<shape type='space' param='");
            append_raw_mixed_length(output, width);
            output.push_str("'>");
        }
    }
}

fn append_html_color(output: &mut String, color: Color) {
    output.push('#');
    let _ = write!(
        output,
        "{:02X}{:02X}{:02X}",
        color.red, color.green, color.blue
    );
}

fn append_presentation_length(
    output: &mut String,
    value: &PresentationLength,
    line_height: LogicalLength,
) {
    match value {
        PresentationLength::Logical(LogicalLength(value)) => {
            output.push_str(&(value / 1_000).to_string());
            output.push_str("px");
        }
        PresentationLength::FontHeightHundredths(value) => {
            let pixels = value.saturating_mul(line_height.0) / 100_000;
            output.push_str(&pixels.to_string());
        }
    }
}

fn append_raw_mixed_length(output: &mut String, value: &PresentationLength) {
    match value {
        PresentationLength::Logical(LogicalLength(value)) => {
            output.push_str(&(value / 1_000).to_string());
            output.push_str("px");
        }
        PresentationLength::FontHeightHundredths(value) => output.push_str(&value.to_string()),
    }
}
