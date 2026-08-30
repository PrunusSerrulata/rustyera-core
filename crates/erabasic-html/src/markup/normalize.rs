use super::{
    HtmlAlignment, HtmlAttribute, HtmlBoxModel, HtmlColorMatrix, HtmlDisplayMode, HtmlElementKind,
    HtmlElementSemantic, HtmlError, HtmlErrorKind, HtmlFontEdging, HtmlFontHinting, HtmlLength,
    HtmlTextRenderIntent, HtmlTextRenderer, HtmlVerticalAlignment, error,
};
#[allow(clippy::too_many_lines)]
pub(super) fn normalize_element(
    kind: HtmlElementKind,
    attributes: &[HtmlAttribute],
    start: usize,
    end: usize,
) -> Result<HtmlElementSemantic, HtmlError> {
    let invalid = || error(HtmlErrorKind::InvalidAttributeValue, start, end);
    let missing = || error(HtmlErrorKind::MissingAttribute, start, end);
    let value = |name: &str| {
        attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    };
    let allowed = |names: &[&str]| {
        attributes
            .iter()
            .all(|attribute| names.contains(&attribute.name.as_str()))
    };
    let no_attributes = || {
        if attributes.is_empty() {
            Ok(())
        } else {
            Err(error(HtmlErrorKind::InvalidAttribute, start, end))
        }
    };

    Ok(match kind {
        HtmlElementKind::Bold
        | HtmlElementKind::Italic
        | HtmlElementKind::Underline
        | HtmlElementKind::Strike => {
            no_attributes()?;
            HtmlElementSemantic::Style
        }
        HtmlElementKind::Break => {
            no_attributes()?;
            HtmlElementSemantic::Break
        }
        HtmlElementKind::NoBreak => {
            no_attributes()?;
            HtmlElementSemantic::NoBreak
        }
        HtmlElementKind::Paragraph => {
            if !allowed(&["align"]) || attributes.len() != 1 {
                return Err(missing());
            }
            let alignment = match value("align")
                .ok_or_else(missing)?
                .to_ascii_lowercase()
                .as_str()
            {
                "left" => HtmlAlignment::Left,
                "center" => HtmlAlignment::Center,
                "right" => HtmlAlignment::Right,
                _ => return Err(invalid()),
            };
            HtmlElementSemantic::Paragraph { alignment }
        }
        HtmlElementKind::Font => {
            if attributes.is_empty()
                || !allowed(&[
                    "face", "color", "bcolor", "size", "valign", "render", "edging", "hinting",
                ])
            {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            HtmlElementSemantic::Font {
                face: value("face").map(str::to_owned),
                color: value("color")
                    .map(parse_color)
                    .transpose()
                    .map_err(|()| invalid())?,
                button_color: value("bcolor")
                    .map(parse_color)
                    .transpose()
                    .map_err(|()| invalid())?,
                size_millipixels: value("size")
                    .map(parse_positive_millipixels)
                    .transpose()
                    .map_err(|()| invalid())?,
                vertical_alignment: value("valign")
                    .map(parse_vertical_alignment)
                    .transpose()
                    .map_err(|()| invalid())?,
                render_intent: HtmlTextRenderIntent {
                    renderer: value("render")
                        .map(parse_text_renderer)
                        .transpose()
                        .map_err(|()| invalid())?,
                    edging: value("edging")
                        .map(parse_font_edging)
                        .transpose()
                        .map_err(|()| invalid())?,
                    hinting: value("hinting")
                        .map(parse_font_hinting)
                        .transpose()
                        .map_err(|()| invalid())?,
                },
            }
        }
        HtmlElementKind::Button => {
            if !allowed(&["value", "title", "pos"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            HtmlElementSemantic::Button {
                value: value("value").map(str::to_owned),
                title: value("title").map(str::to_owned),
                position: value("pos")
                    .map(str::parse)
                    .transpose()
                    .map_err(|_| invalid())?,
            }
        }
        HtmlElementKind::NonButton => {
            if !allowed(&["title", "pos"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            HtmlElementSemantic::NonButton {
                title: value("title").map(str::to_owned),
                position: value("pos")
                    .map(str::parse)
                    .transpose()
                    .map_err(|_| invalid())?,
            }
        }
        HtmlElementKind::ClearButton => {
            if !allowed(&["notooltip"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            let suppress_tooltip = match value("notooltip") {
                None | Some("false" | "FALSE" | "False") => false,
                Some("true" | "TRUE" | "True") => true,
                Some(_) => return Err(invalid()),
            };
            HtmlElementSemantic::ClearButton { suppress_tooltip }
        }
        HtmlElementKind::Image => {
            if !allowed(&[
                "src", "srcb", "srcm", "height", "width", "ypos", "xpos", "display", "cm",
            ]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            HtmlElementSemantic::Image {
                source: value("src").ok_or_else(missing)?.to_owned(),
                hover_source: value("srcb").map(str::to_owned),
                mask_source: value("srcm").map(str::to_owned),
                height: value("height")
                    .map(parse_length)
                    .transpose()
                    .map_err(|()| invalid())?,
                width: value("width")
                    .map(parse_length)
                    .transpose()
                    .map_err(|()| invalid())?,
                y: value("ypos")
                    .map(parse_length)
                    .transpose()
                    .map_err(|()| invalid())?,
                x: value("xpos")
                    .map(parse_length)
                    .transpose()
                    .map_err(|()| invalid())?,
                display: value("display")
                    .map(parse_display_mode)
                    .transpose()
                    .map_err(|()| invalid())?
                    .unwrap_or(HtmlDisplayMode::Relative),
                color_matrix: value("cm")
                    .map(parse_color_matrix_reference)
                    .transpose()
                    .map_err(|()| invalid())?,
            }
        }
        HtmlElementKind::Shape => {
            if !allowed(&["type", "param", "color", "bcolor"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            let parameters = value("param")
                .ok_or_else(missing)?
                .split(',')
                .map(|item| parse_length(item.trim()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|()| invalid())?;
            HtmlElementSemantic::Shape {
                kind: value("type").ok_or_else(missing)?.to_owned(),
                parameters,
                color: value("color")
                    .map(parse_color)
                    .transpose()
                    .map_err(|()| invalid())?,
                button_color: value("bcolor")
                    .map(parse_color)
                    .transpose()
                    .map_err(|()| invalid())?,
            }
        }
        HtmlElementKind::Division => normalize_division(attributes, start, end)?,
    })
}

fn normalize_division(
    attributes: &[HtmlAttribute],
    start: usize,
    end: usize,
) -> Result<HtmlElementSemantic, HtmlError> {
    let invalid = || error(HtmlErrorKind::InvalidAttributeValue, start, end);
    let mut x = None;
    let mut y = None;
    let mut width = None;
    let mut height = None;
    let mut depth = 0;
    let mut color = None;
    let mut display = HtmlDisplayMode::Relative;
    let mut box_model = HtmlBoxModel::default();
    for attribute in attributes {
        match attribute.name.as_str() {
            "xpos" => x = Some(parse_length(&attribute.value).map_err(|()| invalid())?),
            "ypos" => y = Some(parse_length(&attribute.value).map_err(|()| invalid())?),
            "width" => width = Some(parse_length(&attribute.value).map_err(|()| invalid())?),
            "height" => height = Some(parse_length(&attribute.value).map_err(|()| invalid())?),
            "depth" => depth = attribute.value.parse().map_err(|_| invalid())?,
            "color" => color = Some(parse_color(&attribute.value).map_err(|()| invalid())?),
            "display" => {
                display = parse_division_display_mode(&attribute.value).map_err(|()| invalid())?;
            }
            "size" => {
                let values = parse_lengths::<2>(&attribute.value).map_err(|()| invalid())?;
                width = Some(values[0]);
                height = Some(values[1]);
            }
            "rect" => {
                let values = parse_lengths::<4>(&attribute.value).map_err(|()| invalid())?;
                x = Some(values[0]);
                y = Some(values[1]);
                width = Some(values[2]);
                height = Some(values[3]);
            }
            "border" => {
                box_model.border =
                    Some(parse_box_lengths(&attribute.value).map_err(|()| invalid())?);
            }
            "radius" => {
                box_model.radius =
                    Some(parse_box_lengths(&attribute.value).map_err(|()| invalid())?);
            }
            "margin" => {
                box_model.margin =
                    Some(parse_box_lengths(&attribute.value).map_err(|()| invalid())?);
            }
            "padding" => {
                box_model.padding =
                    Some(parse_box_lengths(&attribute.value).map_err(|()| invalid())?);
            }
            "bcolor" => {
                box_model.border_colors =
                    Some(parse_box_colors(&attribute.value).map_err(|()| invalid())?);
            }
            _ => return Err(error(HtmlErrorKind::InvalidAttribute, start, end)),
        }
    }
    Ok(HtmlElementSemantic::Division {
        x,
        y,
        width: width.ok_or_else(|| error(HtmlErrorKind::MissingAttribute, start, end))?,
        height,
        depth,
        color,
        display,
        box_model,
    })
}

fn parse_length(value: &str) -> Result<HtmlLength, ()> {
    if let Some(value) = value
        .strip_suffix("px")
        .or_else(|| value.strip_suffix("PX"))
        .or_else(|| value.strip_suffix("Px"))
        .or_else(|| value.strip_suffix("pX"))
    {
        value.parse().map(HtmlLength::Pixels).map_err(|_| ())
    } else {
        value
            .parse()
            .map(HtmlLength::FontHeightHundredths)
            .map_err(|_| ())
    }
}

fn strip_ascii_case_suffix<'a>(value: &'a str, suffix: &str) -> &'a str {
    value
        .get(value.len().saturating_sub(suffix.len())..)
        .filter(|tail| tail.eq_ignore_ascii_case(suffix))
        .and_then(|_| value.get(..value.len().saturating_sub(suffix.len())))
        .unwrap_or(value)
}

fn parse_positive_millipixels(value: &str) -> Result<u32, ()> {
    let value = strip_ascii_case_suffix(value.trim(), "px");
    let value = value.strip_prefix('+').unwrap_or(value);
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(());
    }
    let whole = if whole.is_empty() {
        0
    } else {
        whole.parse::<u64>().map_err(|_| ())?
    };
    let round = u64::from(
        fraction
            .as_bytes()
            .get(3)
            .is_some_and(|digit| *digit >= b'5'),
    );
    let mut fraction_digits = fraction.bytes().take(3).collect::<Vec<_>>();
    while fraction_digits.len() < 3 {
        fraction_digits.push(b'0');
    }
    let fraction_millipixels = fraction_digits
        .into_iter()
        .fold(0_u64, |value, digit| value * 10 + u64::from(digit - b'0'));
    let millipixels = whole
        .checked_mul(1_000)
        .and_then(|whole| whole.checked_add(fraction_millipixels))
        .and_then(|value| value.checked_add(round))
        .ok_or(())?;
    u32::try_from(millipixels)
        .map_err(|_| ())
        .and_then(|value| if value == 0 { Err(()) } else { Ok(value) })
}

fn parse_vertical_alignment(value: &str) -> Result<HtmlVerticalAlignment, ()> {
    match value.to_ascii_lowercase().as_str() {
        "top" => Ok(HtmlVerticalAlignment::Top),
        "middle" => Ok(HtmlVerticalAlignment::Middle),
        "bottom" => Ok(HtmlVerticalAlignment::Bottom),
        _ => Err(()),
    }
}

fn parse_text_renderer(value: &str) -> Result<HtmlTextRenderer, ()> {
    match value.to_ascii_lowercase().as_str() {
        "gdi" => Ok(HtmlTextRenderer::Gdi),
        "skia" => Ok(HtmlTextRenderer::Skia),
        _ => Err(()),
    }
}

fn parse_font_edging(value: &str) -> Result<HtmlFontEdging, ()> {
    match value.to_ascii_lowercase().as_str() {
        "alias" => Ok(HtmlFontEdging::Alias),
        "antialias" => Ok(HtmlFontEdging::AntiAlias),
        "subpixel" => Ok(HtmlFontEdging::SubpixelAntiAlias),
        _ => Err(()),
    }
}

fn parse_font_hinting(value: &str) -> Result<HtmlFontHinting, ()> {
    match value.to_ascii_lowercase().as_str() {
        "none" => Ok(HtmlFontHinting::None),
        "slight" => Ok(HtmlFontHinting::Slight),
        "normal" => Ok(HtmlFontHinting::Normal),
        "full" => Ok(HtmlFontHinting::Full),
        _ => Err(()),
    }
}

fn parse_display_mode(value: &str) -> Result<HtmlDisplayMode, ()> {
    match value.to_ascii_lowercase().as_str() {
        "relative" => Ok(HtmlDisplayMode::Relative),
        "absolute-lefttop" => Ok(HtmlDisplayMode::AbsoluteLeftTop),
        "absolute-leftbottom" => Ok(HtmlDisplayMode::AbsoluteLeftBottom),
        _ => Err(()),
    }
}

fn parse_color_matrix_reference(value: &str) -> Result<HtmlColorMatrix, ()> {
    let mut parts = value.trim().split(':');
    let name = parts.next().filter(|name| {
        let mut characters = name.chars();
        characters
            .next()
            .is_some_and(|first| first == '_' || first.is_alphabetic())
            && characters.all(|character| character == '_' || character.is_alphanumeric())
    });
    let Some(name) = name else {
        return Err(());
    };
    let mut indices = [0_u64; 3];
    for index in &mut indices {
        let Some(value) = parts.next() else {
            break;
        };
        *index = value.parse().map_err(|_| ())?;
    }
    if parts.next().is_some() {
        return Err(());
    }
    Ok(HtmlColorMatrix::Variable {
        name: name.to_owned(),
        indices,
    })
}

fn parse_division_display_mode(value: &str) -> Result<HtmlDisplayMode, ()> {
    match value.to_ascii_lowercase().as_str() {
        "absolute" => Ok(HtmlDisplayMode::Absolute),
        "absolute-lefttop" => Ok(HtmlDisplayMode::AbsoluteLeftTop),
        other => parse_display_mode(other),
    }
}

fn parse_lengths<const N: usize>(value: &str) -> Result<[HtmlLength; N], ()> {
    let values = value
        .split(',')
        .map(|item| parse_length(item.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    values.try_into().map_err(|_| ())
}

fn expand_four<T: Copy>(values: &[T]) -> Result<[T; 4], ()> {
    Ok(match values {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return Err(()),
    })
}

fn parse_box_lengths(value: &str) -> Result<[HtmlLength; 4], ()> {
    let values = value
        .split(',')
        .map(|item| parse_length(item.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    expand_four(&values)
}

fn parse_box_colors(value: &str) -> Result<[u32; 4], ()> {
    let values = value
        .split(',')
        .map(|item| parse_color(item.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    expand_four(&values)
}

fn parse_color(value: &str) -> Result<u32, ()> {
    if let Some(hex) = value.strip_prefix('#') {
        let color = u32::from_str_radix(hex, 16).map_err(|_| ())?;
        return (color <= 0x00ff_ffff).then_some(color).ok_or(());
    }
    crate::named_color(value).ok_or(())
}

pub(super) fn decode_entities(source: &str, base: usize) -> Result<String, HtmlError> {
    let mut output = String::new();
    super::super::unescape_into(source, &mut output)
        .map_err(|_| error(HtmlErrorKind::InvalidEntity, base, base + source.len()))?;
    Ok(output)
}
