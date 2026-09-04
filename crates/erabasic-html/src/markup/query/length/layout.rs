use super::geometry::{Fraction, add_pixels, bounded_pixels, integer_length, text_pixels};
use super::{
    Entry, HtmlLength, HtmlLengthCut, HtmlLengthImageResolution, HtmlLengthMeasuredValue,
    HtmlLengthProbeKind, HtmlQueryError, HtmlQueryErrorKind, HtmlStringLengthPlan, Part, PartKind,
    error, geometry, invalid_measurement, resource_limit, suffix,
};
use std::collections::VecDeque;

#[derive(Clone)]
struct MeasuredPart {
    part: usize,
    probe: usize,
    point: i64,
    width: i64,
    residual: Fraction,
    utf16_length: usize,
    prefix_end: usize,
}

#[derive(Clone)]
struct MeasuredButton {
    parts: Vec<MeasuredPart>,
    point: i64,
    width: i64,
    residual: Fraction,
    clickable: bool,
    locked: bool,
}

pub(super) struct CompletedLayout {
    pub(super) first_line_pixels: i64,
    pub(super) lines: usize,
}

/// None is a continuation wait for one newly appended suffix probe.
pub(super) fn complete(
    plan: &mut HtmlStringLengthPlan,
    layout_index: usize,
) -> Result<Option<CompletedLayout>, HtmlQueryError> {
    let layout = plan
        .layouts
        .get(layout_index)
        .ok_or_else(invalid_measurement)?;
    let work = layout.entries.iter().fold(0_usize, |sum, entry| {
        sum.saturating_add(match entry {
            Entry::Button(button) => button.parts.len().saturating_mul(256).saturating_add(128),
            Entry::Break => 128,
        })
    });
    plan.charge_work(work)?;
    let layout = plan.layouts[layout_index].clone();
    let (mut point, mut residual) = (0, Fraction::ZERO);
    let mut buttons = VecDeque::with_capacity(layout.entries.len());
    // setWidthToButtonList runs over the complete list before selecting any row.
    // A forced break resets PointX, but deliberately does not reset subPixel.
    for entry in &layout.entries {
        let Entry::Button(button) = entry else {
            point = 0;
            buttons.push_back(None);
            continue;
        };
        let mut measured = MeasuredButton {
            parts: Vec::with_capacity(button.parts.len()),
            point: 0,
            width: 0,
            residual,
            clickable: button.clickable,
            locked: button.position.is_some(),
        };
        for index in &button.parts {
            measured.parts.push(initial_part(plan, *index)?);
        }
        calc_width(plan, &mut measured, residual)?;
        let start = button
            .position
            .map(|position| {
                integer_length(
                    HtmlLength::FontHeightHundredths(position),
                    plan.settings.font_size_pixels,
                )
            })
            .transpose()?
            .unwrap_or(point);
        measured.point = start;
        calc_points(plan, &mut measured, start)?;
        point = add_pixels(measured.point, measured.width)?;
        residual = measured.residual;
        buttons.push_back(Some(measured));
    }
    let (mut sum, mut count, mut first, mut lines) = (0, 0, None, 0);
    while let Some(button) = buttons.pop_front() {
        plan.charge_work(128)?;
        let Some(button) = button else {
            emit_line(plan, &mut sum, &mut count, &mut first, &mut lines)?;
            continue;
        };
        if layout.no_break || add_pixels(button.point, button.width)? <= layout.width {
            sum = add_pixels(sum, button.width)?;
            count += 1;
            continue;
        }
        let split = !plan.settings.prevent_button_wrap
            || count == 0
            || (!button.clickable && !plan.settings.legacy_nonbutton_wrap);
        if split {
            let divide_index = divide_index(plan, &button, layout.width)?;
            if divide_index > 0 {
                let Some((head, tail)) = split_button(plan, &button, divide_index)? else {
                    return Ok(None);
                };
                sum = add_pixels(sum, head.width)?;
                count += 1;
                // DivideAt may return null at the full end; reference inserts that
                // explicit break as well as flushing the current row below.
                buttons.push_front(tail);
                emit_line(plan, &mut sum, &mut count, &mut first, &mut lines)?;
                shift_remaining(plan, &mut buttons)?;
                continue;
            }
            if count == 0 {
                // An indivisible first element is kept even when it exceeds the row.
                sum = add_pixels(sum, button.width)?;
                count += 1;
                continue;
            }
        }
        buttons.push_front(Some(button));
        emit_line(plan, &mut sum, &mut count, &mut first, &mut lines)?;
        shift_remaining(plan, &mut buttons)?;
    }
    if count > 0 {
        emit_line(plan, &mut sum, &mut count, &mut first, &mut lines)?;
    }
    Ok(Some(CompletedLayout {
        first_line_pixels: first.unwrap_or(0),
        lines,
    }))
}

fn emit_line(
    plan: &HtmlStringLengthPlan,
    sum: &mut i64,
    count: &mut usize,
    first: &mut Option<i64>,
    lines: &mut usize,
) -> Result<(), HtmlQueryError> {
    *lines = lines.checked_add(1).ok_or_else(resource_limit)?;
    if *lines > plan.limits.maximum_lines {
        return Err(resource_limit());
    }
    if first.is_none() {
        *first = Some(*sum);
    }
    *sum = 0;
    *count = 0;
    Ok(())
}

fn shift_remaining(
    plan: &mut HtmlStringLengthPlan,
    buttons: &mut VecDeque<Option<MeasuredButton>>,
) -> Result<(), HtmlQueryError> {
    let mut point = 0;
    for button in buttons {
        let Some(button) = button else {
            break;
        };
        plan.charge_work(button.parts.len().saturating_mul(128).saturating_add(64))?;
        calc_points(plan, button, point)?;
        // Reference's post-wrap loop advances by Width, not PointX + Width.
        point = add_pixels(point, button.width)?;
    }
    Ok(())
}

fn value<'a>(
    plan: &'a HtmlStringLengthPlan,
    part: &Part,
) -> Result<&'a HtmlLengthMeasuredValue, HtmlQueryError> {
    plan.values
        .get(part.probe)
        .and_then(Option::as_ref)
        .ok_or_else(invalid_measurement)
}

fn text_data(
    plan: &HtmlStringLengthPlan,
    probe: usize,
) -> Result<(&[HtmlLengthCut], &[i64]), HtmlQueryError> {
    let HtmlLengthProbeKind::TextPart { cuts, .. } =
        &plan.probes.get(probe).ok_or_else(invalid_measurement)?.kind
    else {
        return Err(invalid_measurement());
    };
    let HtmlLengthMeasuredValue::TextPart {
        prefix_advances_millipixels,
    } = plan
        .values
        .get(probe)
        .and_then(Option::as_ref)
        .ok_or_else(invalid_measurement)?
    else {
        return Err(invalid_measurement());
    };
    Ok((cuts, prefix_advances_millipixels))
}

fn initial_part(plan: &HtmlStringLengthPlan, index: usize) -> Result<MeasuredPart, HtmlQueryError> {
    let part = plan.parts.get(index).ok_or_else(invalid_measurement)?;
    let mut measured = MeasuredPart {
        part: index,
        probe: part.probe,
        point: 0,
        width: 0,
        residual: Fraction::ZERO,
        utf16_length: 0,
        prefix_end: 0,
    };
    match &part.kind {
        PartKind::Text => {
            let (cuts, _) = text_data(plan, measured.probe)?;
            measured.prefix_end = cuts.len().checked_sub(1).ok_or_else(invalid_measurement)?;
            measured.utf16_length = cuts.last().ok_or_else(invalid_measurement)?.decoded_utf16;
        }
        PartKind::Fallback { utf16_length } => measured.utf16_length = *utf16_length,
        PartKind::Image {
            height,
            width,
            fallback_utf16_length,
        } => {
            let HtmlLengthMeasuredValue::ImageSlot(resolution) = value(plan, part)? else {
                return Err(invalid_measurement());
            };
            match resolution {
                HtmlLengthImageResolution::Loaded {
                    natural_width,
                    natural_height,
                } => {
                    (measured.width, measured.residual) = geometry::image_width(
                        *height,
                        *width,
                        *natural_width,
                        *natural_height,
                        plan.settings,
                    )?;
                }
                HtmlLengthImageResolution::Missing { .. } => {
                    measured.utf16_length = *fallback_utf16_length;
                }
            }
        }
        PartKind::Shape { .. } | PartKind::Division { .. } => {}
    }
    Ok(measured)
}

fn calc_width(
    plan: &HtmlStringLengthPlan,
    button: &mut MeasuredButton,
    mut residual: Fraction,
) -> Result<(), HtmlQueryError> {
    for measured in &mut button.parts {
        let part = plan
            .parts
            .get(measured.part)
            .ok_or_else(invalid_measurement)?;
        if measured.width <= 0 {
            match &part.kind {
                PartKind::Text => {
                    let (_, prefixes) = text_data(plan, measured.probe)?;
                    measured.width = text_pixels(
                        *prefixes
                            .get(measured.prefix_end)
                            .ok_or_else(invalid_measurement)?,
                    )?;
                    measured.residual = residual;
                }
                PartKind::Shape { advance } => {
                    (measured.width, measured.residual) = advance.add(residual)?.split()?;
                }
                PartKind::Fallback { .. } => {
                    let HtmlLengthMeasuredValue::FallbackText {
                        advance_millipixels,
                    } = value(plan, part)?
                    else {
                        return Err(invalid_measurement());
                    };
                    measured.width = text_pixels(*advance_millipixels)?;
                    measured.residual = residual;
                }
                PartKind::Image { .. } => match value(plan, part)? {
                    HtmlLengthMeasuredValue::ImageSlot(HtmlLengthImageResolution::Missing {
                        fallback_advance_millipixels,
                    }) => {
                        measured.width = text_pixels(*fallback_advance_millipixels)?;
                        measured.residual = residual;
                    }
                    HtmlLengthMeasuredValue::ImageSlot(HtmlLengthImageResolution::Loaded {
                        ..
                    }) => {}
                    _ => return Err(invalid_measurement()),
                },
                PartKind::Division { .. } => {}
            }
        }
        residual = measured.residual;
    }
    button.residual = residual;
    Ok(())
}

fn calc_points(
    plan: &HtmlStringLengthPlan,
    button: &mut MeasuredButton,
    mut point: i64,
) -> Result<(), HtmlQueryError> {
    if button.locked {
        point = button.point;
    }
    for measured in &mut button.parts {
        if matches!(
            plan.parts
                .get(measured.part)
                .ok_or_else(invalid_measurement)?
                .kind,
            PartKind::Division { absolute: true }
        ) {
            continue;
        }
        measured.point = point;
        point = add_pixels(point, measured.width)?;
    }
    let first = button.parts.first().ok_or_else(invalid_measurement)?;
    let last = button.parts.last().ok_or_else(invalid_measurement)?;
    button.point = first.point;
    // This is neither max bbox nor a simple sum when absolute divs are at an edge.
    button.width = bounded_pixels(
        add_pixels(last.point, last.width)?
            .checked_sub(first.point)
            .ok_or_else(resource_limit)?,
    )?;
    Ok(())
}

fn divide_index(
    plan: &mut HtmlStringLengthPlan,
    button: &MeasuredButton,
    limit: i64,
) -> Result<usize, HtmlQueryError> {
    plan.charge_work(button.parts.len().saturating_mul(128))?;
    let (mut point, mut text_length, mut fitting_parts) = (button.point, 0_usize, 0);
    for measured in &button.parts {
        let part = plan
            .parts
            .get(measured.part)
            .ok_or_else(invalid_measurement)?;
        if add_pixels(point, measured.width)? > limit {
            if fitting_parts == 0 && !matches!(part.kind, PartKind::Text) {
                // Reference does not advance point or text_length in this branch.
                continue;
            }
            if matches!(part.kind, PartKind::Text) {
                let (cuts, widths) = text_data(plan, measured.probe)?;
                let width_limit = limit
                    .checked_sub(measured.point)
                    .ok_or_else(resource_limit)?;
                let (mut fits, mut fails) = (0, measured.prefix_end);
                while fits + 1 < fails {
                    let middle = usize::midpoint(fits, fails);
                    if text_pixels(*widths.get(middle).ok_or_else(invalid_measurement)?)?
                        <= width_limit
                    {
                        fits = middle;
                    } else {
                        fails = middle;
                    }
                }
                text_length = text_length
                    .checked_add(
                        cuts.get(fits)
                            .ok_or_else(invalid_measurement)?
                            .decoded_utf16,
                    )
                    .ok_or_else(resource_limit)?;
            }
            break;
        }
        fitting_parts += 1;
        text_length = text_length
            .checked_add(measured.utf16_length)
            .ok_or_else(resource_limit)?;
        point = add_pixels(point, measured.width)?;
    }
    Ok(text_length)
}

fn split_button(
    plan: &mut HtmlStringLengthPlan,
    button: &MeasuredButton,
    index: usize,
) -> Result<Option<(MeasuredButton, Option<MeasuredButton>)>, HtmlQueryError> {
    plan.charge_work(button.parts.len().saturating_mul(256).saturating_add(128))?;
    let mut head = MeasuredButton {
        parts: Vec::new(),
        point: button.point,
        width: button.width,
        residual: button.residual,
        clickable: button.clickable,
        locked: button.locked,
    };
    let mut tail = MeasuredButton {
        parts: Vec::new(),
        point: 0,
        width: 0,
        residual: Fraction::ZERO,
        clickable: button.clickable,
        locked: false,
    };
    let mut length = 0_usize;
    let mut divided = false;
    for measured in &button.parts {
        if divided {
            tail.parts.push(measured.clone());
            continue;
        }
        let mut part = measured.clone();
        let end = length
            .checked_add(part.utf16_length)
            .ok_or_else(resource_limit)?;
        if index < end {
            let definition = plan.parts.get(part.part).ok_or_else(invalid_measurement)?;
            if !matches!(definition.kind, PartKind::Text) || index <= length {
                return Err(error(
                    HtmlQueryErrorKind::InvalidMarkup,
                    "reference divide index falls inside an indivisible display part",
                ));
            }
            let (cuts, widths) = text_data(plan, part.probe)?;
            let cut = cuts
                .binary_search_by_key(&(index - length), |cut| cut.decoded_utf16)
                .map_err(|_| invalid_measurement())?;
            part.prefix_end = cut;
            part.utf16_length = index - length;
            part.width = text_pixels(
                *widths
                    .get(part.prefix_end)
                    .ok_or_else(invalid_measurement)?,
            )?;
            let suffix = suffix::request(plan, measured.probe, cut)?;
            if plan.values.get(suffix).is_none_or(Option::is_none) {
                return Ok(None);
            }
            let (cuts, widths) = text_data(plan, suffix)?;
            let prefix_end = cuts.len().checked_sub(1).ok_or_else(invalid_measurement)?;
            // ConsoleStyledString.DivideAt measures both the new head and suffix
            // with the old part's XsubPixel before either button recalculates width.
            let suffix_part = MeasuredPart {
                part: measured.part,
                probe: suffix,
                point: 0,
                width: text_pixels(*widths.get(prefix_end).ok_or_else(invalid_measurement)?)?,
                residual: measured.residual,
                utf16_length: cuts.last().ok_or_else(invalid_measurement)?.decoded_utf16,
                prefix_end,
            };
            if suffix_part.utf16_length >= measured.utf16_length || suffix_part.utf16_length == 0 {
                return Err(error(
                    HtmlQueryErrorKind::NoProgress,
                    "HTML layout suffix did not consume text",
                ));
            }
            head.parts.push(part);
            tail.parts.push(suffix_part);
            divided = true;
            continue;
        }
        head.parts.push(part);
        if index == end {
            divided = true;
            continue;
        }
        length = end;
    }
    if tail.parts.is_empty() {
        return Ok(Some((button.clone(), None)));
    }
    calc_width(plan, &mut head, button.residual)?;
    calc_width(plan, &mut tail, Fraction::ZERO)?;
    calc_points(plan, &mut head, button.point)?;
    calc_points(plan, &mut tail, add_pixels(head.point, head.width)?)?;
    Ok(Some((head, Some(tail))))
}
