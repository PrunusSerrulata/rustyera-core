//! Reuse a canonical text probe's style while shaping its suffix independently.
//! No source serialization, markup parsing, or prefix-width subtraction occurs here.

use super::super::super::HtmlNode;
use super::{
    HtmlDocument, HtmlLengthCut, HtmlLengthProbe, HtmlLengthProbeKind, HtmlQueryError,
    HtmlQueryErrorKind, HtmlSourceRange, HtmlStringLengthPlan, error, invalid_measurement,
    resource_limit,
};

pub(super) fn request(
    plan: &mut HtmlStringLengthPlan,
    parent: usize,
    cut: usize,
) -> Result<usize, HtmlQueryError> {
    if let Some(probe) = plan
        .suffix_probes
        .get(&parent)
        .and_then(|suffixes| suffixes.get(&cut))
    {
        return Ok(*probe);
    }
    let original = plan.probes.get(parent).ok_or_else(invalid_measurement)?;
    let HtmlLengthProbeKind::TextPart { cuts, .. } = &original.kind else {
        return Err(invalid_measurement());
    };
    if cut == 0 || cut >= cuts.len().saturating_sub(1) {
        return Err(invalid_measurement());
    }
    let count = cuts.len() - cut;
    let units = plan
        .measurement_units
        .saturating_add(count)
        .saturating_add(1);
    if units > plan.limits.maximum_measurements
        || plan.probes.len() >= plan.limits.maximum_measurements
    {
        return Err(resource_limit());
    }
    let work = document_cost(&original.document)
        .saturating_mul(2)
        .saturating_add(
            count.saturating_mul(std::mem::size_of::<HtmlLengthCut>() + std::mem::size_of::<i64>()),
        )
        .saturating_add(4096);
    // Reserve before copying a potentially long remaining suffix or font-face value.
    plan.charge_work(work)?;
    let original = &plan.probes[parent];
    let HtmlLengthProbeKind::TextPart {
        text_node_path,
        cuts,
    } = &original.kind
    else {
        return Err(invalid_measurement());
    };
    let start = cuts[cut];
    let end = *cuts.last().ok_or_else(invalid_measurement)?;
    let source = HtmlSourceRange {
        start: start.source_byte.unwrap_or(original.source.start),
        end: end.source_byte.unwrap_or(original.source.end),
    };
    let document = HtmlDocument {
        nodes: vec![crop(
            &original.document.nodes,
            text_node_path,
            start.decoded_utf8,
            source,
        )?],
    };
    let rebased = cuts[cut..]
        .iter()
        .map(|boundary| {
            Ok(HtmlLengthCut {
                decoded_utf8: boundary
                    .decoded_utf8
                    .checked_sub(start.decoded_utf8)
                    .ok_or_else(invalid_measurement)?,
                decoded_utf16: boundary
                    .decoded_utf16
                    .checked_sub(start.decoded_utf16)
                    .ok_or_else(invalid_measurement)?,
                source_byte: boundary.source_byte,
            })
        })
        .collect::<Result<Vec<_>, HtmlQueryError>>()?;
    let probe = plan.probes.len();
    let next = HtmlLengthProbe {
        id: probe as u64,
        document,
        kind: HtmlLengthProbeKind::TextPart {
            text_node_path: text_node_path.clone(),
            cuts: rebased,
        },
        source,
    };
    plan.probes.push(next);
    plan.values.push(None);
    plan.suffix_probes
        .entry(parent)
        .or_default()
        .insert(cut, probe);
    plan.measurement_units = units;
    Ok(probe)
}

fn document_cost(document: &HtmlDocument) -> usize {
    let mut pending = document.nodes.iter().collect::<Vec<_>>();
    let mut bytes = 0_usize;
    while let Some(node) = pending.pop() {
        bytes = bytes.saturating_add(256);
        match node {
            HtmlNode::Text { text, .. } => bytes = bytes.saturating_add(text.len()),
            HtmlNode::Element {
                attributes,
                children,
                ..
            } => {
                // Typed semantic strings duplicate some attribute values; charge both.
                for attribute in attributes {
                    bytes = bytes.saturating_add(
                        attribute
                            .name
                            .len()
                            .saturating_add(attribute.value.len())
                            .saturating_mul(2),
                    );
                }
                pending.extend(children);
            }
        }
    }
    bytes
}

fn crop(
    nodes: &[HtmlNode],
    path: &[usize],
    utf8: usize,
    range: HtmlSourceRange,
) -> Result<HtmlNode, HtmlQueryError> {
    if nodes.len() != 1 || path.first() != Some(&0) {
        return Err(invalid_measurement());
    }
    match &nodes[0] {
        HtmlNode::Text { text, .. } if path.len() == 1 => {
            let suffix = text
                .get(utf8..)
                .filter(|suffix| !suffix.is_empty() && suffix.len() < text.len())
                .ok_or_else(|| {
                    error(
                        HtmlQueryErrorKind::NoProgress,
                        "HTML layout suffix has no legal advancing scalar boundary",
                    )
                })?;
            Ok(HtmlNode::Text {
                text: suffix.into(),
                start: range.start as u64,
                end: range.end as u64,
            })
        }
        HtmlNode::Element {
            kind,
            attributes,
            children,
            semantic,
            ..
        } if path.len() > 1 => {
            let child = crop(children, &path[1..], utf8, range)?;
            Ok(HtmlNode::Element {
                kind: *kind,
                attributes: attributes.clone(),
                children: vec![child],
                interaction: None,
                start: range.start as u64,
                end: range.end as u64,
                semantic: semantic.clone(),
            })
        }
        _ => Err(invalid_measurement()),
    }
}
