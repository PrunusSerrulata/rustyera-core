//! Strict v2 probe mapping. Prefix chunks retain the same whole-part advance.

use era_runtime_protocol::{
    HtmlCutAdvanceV2, HtmlMeasureProbeV2, HtmlMeasureResponseV2, HtmlProbeCutV2, HtmlProbeModeV2,
    HtmlProbeResultV2,
};
use erabasic_html::{
    HtmlLengthImageResolution, HtmlLengthMeasuredValue, HtmlLengthMeasurement, HtmlLengthProbe,
    HtmlLengthProbeKind, HtmlQueryError, HtmlQueryErrorKind,
};
use serde::{Deserialize, Serialize};

use super::plan::{QueryBudget, failure};

const CUTS_PER_REQUEST: usize = 256;
const PREFIX_WORK_PER_REQUEST: usize = 500_000;
const MAXIMUM_ADVANCE: i64 = 1_048_576_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct ProbeTransfer {
    probe: HtmlLengthProbe,
    next_cut: usize,
    values: Vec<i64>,
    whole: Option<i64>,
    expected: Option<HtmlMeasureProbeV2>,
}

impl ProbeTransfer {
    pub(super) fn new(probe: HtmlLengthProbe) -> Self {
        Self {
            probe,
            next_cut: 0,
            values: Vec::new(),
            whole: None,
            expected: None,
        }
    }

    pub(super) fn request(
        &mut self,
        id: u32,
        budget: &mut QueryBudget,
    ) -> Result<HtmlMeasureProbeV2, HtmlQueryError> {
        if self.expected.is_some() {
            return Err(invalid("probe already has an in-flight request"));
        }
        let mut wire = HtmlMeasureProbeV2 {
            id,
            document: self.probe.document.clone(),
            mode: HtmlProbeModeV2::TextPart,
            cuts: Vec::new(),
            missing_document: None,
        };
        match &self.probe.kind {
            HtmlLengthProbeKind::TextPart {
                text_node_path,
                cuts,
            } => {
                let path = text_node_path
                    .iter()
                    .copied()
                    .map(u32::try_from)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| invalid("text path overflow"))?;
                let mut prefix_work = 0usize;
                for (index, cut) in cuts
                    .iter()
                    .enumerate()
                    .skip(self.next_cut)
                    .take(CUTS_PER_REQUEST)
                {
                    let cost = cut.decoded_utf16.saturating_add(path.len());
                    if !wire.cuts.is_empty()
                        && prefix_work.saturating_add(cost) > PREFIX_WORK_PER_REQUEST
                    {
                        break;
                    }
                    if cost > PREFIX_WORK_PER_REQUEST {
                        return Err(limit("single prefix exceeds provider work limit"));
                    }
                    prefix_work += cost;
                    wire.cuts.push(HtmlProbeCutV2 {
                        id: u32::try_from(index).map_err(|_| invalid("cut identifier overflow"))?,
                        text_node_path: path.clone(),
                        decoded_utf8_offset: u32::try_from(cut.decoded_utf8)
                            .map_err(|_| invalid("UTF-8 offset overflow"))?,
                        decoded_utf16_offset: u32::try_from(cut.decoded_utf16)
                            .map_err(|_| invalid("UTF-16 offset overflow"))?,
                    });
                }
                if wire.cuts.is_empty() {
                    return Err(invalid("text probe contains no unmeasured cuts"));
                }
                budget.charge(prefix_work, wire.cuts.len())?;
            }
            HtmlLengthProbeKind::ImageSlot { missing_document } => {
                wire.mode = HtmlProbeModeV2::ImageSlot;
                wire.missing_document = Some(missing_document.clone());
                budget.charge(0, 1)?;
            }
            HtmlLengthProbeKind::FixedSlot => {
                wire.mode = HtmlProbeModeV2::FixedSlot;
                budget.charge(0, 1)?;
            }
            HtmlLengthProbeKind::FallbackText => {
                budget.charge(0, 1)?;
            }
        }
        self.expected = Some(wire.clone());
        Ok(wire)
    }

    pub(super) fn validate_identity(
        &self,
        response: &HtmlMeasureResponseV2,
    ) -> Result<(), HtmlQueryError> {
        let expected = self
            .expected
            .as_ref()
            .ok_or_else(|| invalid("unsolicited probe response"))?;
        if response.probes.len() != 1 || response.probes[0].id != expected.id {
            return Err(invalid("probe identifiers do not match the request"));
        }
        Ok(())
    }

    pub(super) fn resume(
        &mut self,
        response: HtmlMeasureResponseV2,
    ) -> Result<Option<HtmlLengthMeasurement>, HtmlQueryError> {
        self.validate_identity(&response)?;
        let expected = self.expected.take().expect("validated pending transfer");
        let result = response
            .probes
            .into_iter()
            .next()
            .expect("one probe")
            .result;
        if let HtmlProbeResultV2::Error { error } = &result {
            let kind = if error.code == "resource_limit" {
                HtmlQueryErrorKind::ResourceLimit
            } else {
                HtmlQueryErrorKind::InvalidMeasurement
            };
            // Preserve the frontend's stable error category in the diagnostic.
            return Err(failure(kind, &format!("{}: {}", error.code, error.message)));
        }
        let value = match (&self.probe.kind, result) {
            (
                HtmlLengthProbeKind::TextPart { cuts: all, .. },
                HtmlProbeResultV2::TextMeasured {
                    advance_millipixels,
                    cuts,
                },
            ) => {
                let cut_count = all.len();
                let Some(value) =
                    self.resume_text_part(&expected.cuts, cut_count, advance_millipixels, cuts)?
                else {
                    return Ok(None);
                };
                value
            }
            (
                HtmlLengthProbeKind::FallbackText,
                HtmlProbeResultV2::TextMeasured {
                    advance_millipixels,
                    cuts,
                },
            ) => {
                valid_advance(advance_millipixels)?;
                if !cuts.is_empty() {
                    return Err(invalid("fallback text returned unexpected cuts"));
                }
                HtmlLengthMeasuredValue::FallbackText {
                    advance_millipixels,
                }
            }
            (
                HtmlLengthProbeKind::ImageSlot { .. },
                HtmlProbeResultV2::ImageLoaded {
                    natural_width,
                    natural_height,
                },
            ) => {
                if natural_width == 0
                    || natural_height == 0
                    || natural_width > 1_048_576
                    || natural_height > 1_048_576
                {
                    return Err(invalid("invalid sprite destination-base dimensions"));
                }
                HtmlLengthMeasuredValue::ImageSlot(HtmlLengthImageResolution::Loaded {
                    natural_width,
                    natural_height,
                })
            }
            (
                HtmlLengthProbeKind::ImageSlot { .. },
                HtmlProbeResultV2::ImageMissing {
                    fallback_advance_millipixels,
                },
            ) => {
                valid_advance(fallback_advance_millipixels)?;
                HtmlLengthMeasuredValue::ImageSlot(HtmlLengthImageResolution::Missing {
                    fallback_advance_millipixels,
                })
            }
            (HtmlLengthProbeKind::FixedSlot, HtmlProbeResultV2::FixedReady) => {
                HtmlLengthMeasuredValue::FixedSlotReady
            }
            _ => return Err(invalid("HTML response mode does not match requested probe")),
        };
        Ok(Some(HtmlLengthMeasurement {
            probe_id: self.probe.id,
            value,
        }))
    }

    fn resume_text_part(
        &mut self,
        expected: &[HtmlProbeCutV2],
        cut_count: usize,
        advance_millipixels: i64,
        cuts: Vec<HtmlCutAdvanceV2>,
    ) -> Result<Option<HtmlLengthMeasuredValue>, HtmlQueryError> {
        valid_advance(advance_millipixels)?;
        if self
            .whole
            .is_some_and(|previous| previous != advance_millipixels)
        {
            return Err(invalid("whole-part advance changed between prefix chunks"));
        }
        self.whole = Some(advance_millipixels);
        let values = matching_cuts(expected, cuts)?;
        for (cut, value) in expected.iter().zip(&values) {
            if cut.decoded_utf8_offset == 0 && *value != 0 {
                return Err(invalid("empty prefix has a nonzero advance"));
            }
        }
        self.next_cut = self
            .next_cut
            .checked_add(values.len())
            .ok_or_else(|| invalid("cut progress overflow"))?;
        self.values.extend(values);
        if self.next_cut < cut_count {
            return Ok(None);
        }
        if self.next_cut != cut_count || self.values.last() != Some(&advance_millipixels) {
            return Err(invalid("full prefix differs from whole-part advance"));
        }
        Ok(Some(HtmlLengthMeasuredValue::TextPart {
            prefix_advances_millipixels: std::mem::take(&mut self.values),
        }))
    }
}

fn matching_cuts(
    expected: &[HtmlProbeCutV2],
    actual: Vec<HtmlCutAdvanceV2>,
) -> Result<Vec<i64>, HtmlQueryError> {
    if actual.len() != expected.len() {
        return Err(invalid("HTML response cut count mismatch"));
    }
    let mut indexed = std::collections::BTreeMap::new();
    for cut in actual {
        valid_advance(cut.advance_millipixels)?;
        if indexed.insert(cut.id, cut.advance_millipixels).is_some() {
            return Err(invalid("duplicate HTML response cut"));
        }
    }
    expected
        .iter()
        .map(|cut| {
            indexed
                .remove(&cut.id)
                .ok_or_else(|| invalid("missing HTML response cut"))
        })
        .collect()
}

fn valid_advance(value: i64) -> Result<(), HtmlQueryError> {
    if !(0..=MAXIMUM_ADVANCE).contains(&value) {
        return Err(invalid("invalid HTML millipixel advance"));
    }
    Ok(())
}

fn invalid(message: &str) -> HtmlQueryError {
    failure(HtmlQueryErrorKind::InvalidMeasurement, message)
}
fn limit(message: &str) -> HtmlQueryError {
    failure(HtmlQueryErrorKind::ResourceLimit, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use era_runtime_protocol::{HtmlProbeResponseV2, ProjectionQueryContext};

    fn transfer(text: &str) -> ProbeTransfer {
        let settings = erabasic_html::HtmlStringLengthSettings {
            font_size_pixels: 18,
            drawable_width_pixels: 1000,
            prevent_button_wrap: true,
            legacy_nonbutton_wrap: false,
            foreground_rgb: 0x00ff_ffff,
            focus_rgb: 0x00ff_ffff,
        };
        let plan = erabasic_html::HtmlStringLengthPlan::new(
            text,
            settings,
            1,
            super::super::plan::limits(),
        )
        .unwrap();
        ProbeTransfer::new(plan.probes()[0].clone())
    }

    fn response(request: &HtmlMeasureProbeV2, whole: i64) -> HtmlMeasureResponseV2 {
        HtmlMeasureResponseV2 {
            context: ProjectionQueryContext {
                presentation_revision: 1,
                environment_revision: 1,
                projection_space_revision: 1,
            },
            probes: vec![HtmlProbeResponseV2 {
                id: request.id,
                result: HtmlProbeResultV2::TextMeasured {
                    advance_millipixels: whole,
                    cuts: request
                        .cuts
                        .iter()
                        .map(|cut| HtmlCutAdvanceV2 {
                            id: cut.id,
                            advance_millipixels: i64::from(cut.decoded_utf16_offset) * 1000,
                        })
                        .collect(),
                },
            }],
        }
    }

    #[test]
    fn prefix_transfer_chunks_without_losing_ordinal_or_whole_width() {
        let mut transfer = transfer(&"a".repeat(300));
        let mut budget = QueryBudget::default();
        let first = transfer.request(1, &mut budget).unwrap();
        assert_eq!(first.cuts.len(), 256);
        assert!(
            transfer
                .resume(response(&first, 300_000))
                .unwrap()
                .is_none()
        );
        let second = transfer.request(2, &mut budget).unwrap();
        assert_eq!(second.cuts[0].id, 256);
        let value = transfer
            .resume(response(&second, 300_000))
            .unwrap()
            .unwrap();
        let HtmlLengthMeasuredValue::TextPart {
            prefix_advances_millipixels,
        } = value.value
        else {
            panic!("text result")
        };
        assert_eq!(prefix_advances_millipixels.len(), 301);
        assert_eq!(prefix_advances_millipixels[300], 300_000);
        assert!(budget.measurements >= 301);
    }

    #[test]
    fn prefix_transfer_rejects_changed_whole_duplicate_missing_and_bad_mode() {
        let mut transfer = transfer(&"a".repeat(300));
        let mut budget = QueryBudget::default();
        let first = transfer.request(1, &mut budget).unwrap();
        transfer.resume(response(&first, 300_000)).unwrap();
        let second = transfer.request(2, &mut budget).unwrap();
        assert!(transfer.resume(response(&second, 301_000)).is_err());
        for mutation in ["duplicate", "missing", "negative", "mode", "id"] {
            let mut transfer = self::transfer("ab");
            let request = transfer.request(1, &mut QueryBudget::default()).unwrap();
            let mut reply = response(&request, 2000);
            match mutation {
                "mode" => reply.probes[0].result = HtmlProbeResultV2::FixedReady,
                "id" => reply.probes[0].id += 1,
                _ => {
                    let HtmlProbeResultV2::TextMeasured { cuts, .. } = &mut reply.probes[0].result
                    else {
                        unreachable!()
                    };
                    match mutation {
                        "duplicate" => cuts[1].id = cuts[0].id,
                        "missing" => {
                            cuts.pop();
                        }
                        "negative" => cuts[1].advance_millipixels = -1,
                        _ => unreachable!(),
                    }
                }
            }
            assert!(transfer.resume(reply).is_err(), "{mutation}");
        }
    }
}
