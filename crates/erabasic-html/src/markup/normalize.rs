use super::{
    HtmlAlignment, HtmlAttribute, HtmlBoxModel, HtmlElementKind, HtmlElementSemantic, HtmlError,
    HtmlErrorKind, HtmlLength, error,
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
            if attributes.is_empty() || !allowed(&["face", "color", "bcolor"]) {
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
            if !allowed(&["src", "srcb", "srcm", "height", "width", "ypos"]) {
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
    let mut relative = true;
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
                relative = match attribute.value.to_ascii_lowercase().as_str() {
                    "relative" => true,
                    "absolute" => false,
                    _ => return Err(invalid()),
                };
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
        height: height.ok_or_else(|| error(HtmlErrorKind::MissingAttribute, start, end))?,
        depth,
        color,
        relative,
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
