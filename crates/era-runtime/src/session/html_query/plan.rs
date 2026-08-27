//! Owned query state; no frontend result may choose source text or line boundaries.

pub(super) use erabasic_html::html_string_length_units as reference_length_unit;
use erabasic_html::{
    HtmlLengthMeasurement, HtmlLengthProbe, HtmlQueryError, HtmlQueryErrorKind, HtmlQueryLimits,
    HtmlSourceRange, HtmlStringLengthPlan, HtmlStringLengthPoll, HtmlStringLengthSettings,
    HtmlSubstringPlan, HtmlSubstringPoll, HtmlSubstringResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) enum QueryPlan {
    Length(MeasuredLength),
    Substring {
        split: HtmlSubstringPlan,
        split_usage: (usize, usize),
        nested: Option<(u64, Box<MeasuredLength>)>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct MeasuredLength {
    plan: HtmlStringLengthPlan,
    accounted: (usize, usize),
}

impl MeasuredLength {
    fn account(&mut self, budget: &mut QueryBudget) -> Result<(), HtmlQueryError> {
        let now = self.plan.usage();
        budget.charge(
            now.0.saturating_sub(self.accounted.0),
            now.1.saturating_sub(self.accounted.1),
        )?;
        self.accounted = now;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PlanPoll {
    Measure(HtmlLengthProbe),
    Integer(i64),
    Substring(HtmlSubstringResult),
}

/// This outer budget survives every nested length plan and each lazy LINES step.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct QueryBudget {
    pub(super) work: usize,
    pub(super) measurements: usize,
}

pub(super) fn limits() -> HtmlQueryLimits {
    HtmlQueryLimits {
        maximum_nodes: 4096,
        ..HtmlQueryLimits::default()
    }
}

pub(super) fn failure(kind: HtmlQueryErrorKind, message: &str) -> HtmlQueryError {
    HtmlQueryError {
        kind,
        range: HtmlSourceRange::default(),
        message: message.into(),
    }
}

impl QueryBudget {
    pub(super) fn charge(
        &mut self,
        work: usize,
        measurements: usize,
    ) -> Result<(), HtmlQueryError> {
        let next = Self {
            work: self.work.checked_add(work).ok_or_else(resource_limit)?,
            measurements: self
                .measurements
                .checked_add(measurements)
                .ok_or_else(resource_limit)?,
        };
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub(super) fn validate(&self) -> Result<(), HtmlQueryError> {
        if self.work > limits().maximum_work_bytes
            || self.measurements > limits().maximum_measurements
        {
            return Err(resource_limit());
        }
        Ok(())
    }

    /// Inner plans receive the remaining whole-call allowance, never a fresh full budget.
    pub(super) fn remaining(&self) -> HtmlQueryLimits {
        HtmlQueryLimits {
            maximum_work_bytes: limits().maximum_work_bytes.saturating_sub(self.work),
            maximum_measurements: limits()
                .maximum_measurements
                .saturating_sub(self.measurements),
            ..limits()
        }
    }
}

fn resource_limit() -> HtmlQueryError {
    failure(
        HtmlQueryErrorKind::ResourceLimit,
        "HTML query whole-call work limit exceeded",
    )
}

impl QueryPlan {
    pub(super) fn length(
        source: &str,
        settings: HtmlStringLengthSettings,
        flag: i64,
        budget: &mut QueryBudget,
    ) -> Result<Self, HtmlQueryError> {
        let mut length = MeasuredLength {
            plan: HtmlStringLengthPlan::new(source, settings, flag, budget.remaining())?,
            accounted: (0, 0),
        };
        length.account(budget)?;
        Ok(Self::Length(length))
    }

    pub(super) fn substring(
        source: &str,
        pixels: i64,
        budget: &mut QueryBudget,
    ) -> Result<Self, HtmlQueryError> {
        let split = HtmlSubstringPlan::new(source, pixels, budget.remaining())?;
        let split_usage = split.usage();
        budget.charge(split_usage.0, split_usage.1)?;
        Ok(Self::Substring {
            split,
            split_usage,
            nested: None,
        })
    }

    pub(super) fn poll(
        &mut self,
        settings: HtmlStringLengthSettings,
        budget: &mut QueryBudget,
    ) -> Result<PlanPoll, HtmlQueryError> {
        loop {
            let length = match self {
                Self::Length(plan) => plan,
                Self::Substring {
                    split,
                    split_usage,
                    nested,
                } => {
                    if nested.is_none() {
                        let result = split.poll();
                        let now = split.usage();
                        budget.charge(
                            now.0.saturating_sub(split_usage.0),
                            now.1.saturating_sub(split_usage.1),
                        )?;
                        *split_usage = now;
                        match result? {
                            HtmlSubstringPoll::Complete(result) => {
                                return Ok(PlanPoll::Substring(result));
                            }
                            HtmlSubstringPoll::NeedMeasure(probe) => {
                                // Every scalar/atomic probe obeys the same reference HtmlLength
                                // grouping and slot rules as the complete STRINGLEN operation.
                                let mut length = MeasuredLength {
                                    plan: HtmlStringLengthPlan::from_document(
                                        probe.document,
                                        settings,
                                        1,
                                        budget.remaining(),
                                    )?,
                                    accounted: (0, 0),
                                };
                                length.account(budget)?;
                                *nested = Some((probe.id, Box::new(length)));
                            }
                        }
                    }
                    nested
                        .as_mut()
                        .expect("initialized nested length")
                        .1
                        .as_mut()
                }
            };
            let polled = length.plan.poll();
            length.account(budget)?;
            match polled? {
                HtmlStringLengthPoll::NeedMeasurements { probe_ids } => {
                    let id = probe_ids.first().ok_or_else(|| {
                        failure(
                            HtmlQueryErrorKind::InvalidMeasurement,
                            "length plan requested no probes",
                        )
                    })?;
                    return length
                        .plan
                        .probes()
                        .iter()
                        .find(|probe| probe.id == *id)
                        .cloned()
                        .map(PlanPoll::Measure)
                        .ok_or_else(|| {
                            failure(
                                HtmlQueryErrorKind::InvalidMeasurement,
                                "length plan probe is missing",
                            )
                        });
                }
                HtmlStringLengthPoll::Complete(result) => match self {
                    Self::Length(_) => return Ok(PlanPoll::Integer(result.value)),
                    Self::Substring { split, nested, .. } => {
                        let (id, _) = nested.take().expect("completed nested length");
                        split.resume(id, result.first_line_pixels)?;
                    }
                },
            }
        }
    }

    pub(super) fn resume(&mut self, value: HtmlLengthMeasurement) -> Result<(), HtmlQueryError> {
        let length = match self {
            Self::Length(plan) => plan,
            Self::Substring {
                nested: Some((_, plan)),
                ..
            } => plan.as_mut(),
            Self::Substring { nested: None, .. } => {
                return Err(failure(
                    HtmlQueryErrorKind::InvalidMeasurement,
                    "unsolicited HTML length response",
                ));
            }
        };
        length.plan.resume(value)
    }
}

/// Creator.Method casts the long to int, then `HtmlManager` multiplies unchecked int32.
/// Keep this reference conversion local: it is not the general arithmetic policy.
pub(super) fn reference_split_pixels(width: i64, font_size: i32) -> i64 {
    let bytes = width.to_le_bytes();
    let width = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    i64::from(width.wrapping_mul(font_size) / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_width_conversion_preserves_local_int32_cast_and_unchecked_multiply() {
        assert_eq!(reference_split_pixels(4_294_967_297, 18), 9);
        assert_eq!(reference_split_pixels(i64::MIN, 18), 0);
        assert_eq!(reference_split_pixels(i64::MAX, 18), -9);
        assert_eq!(reference_split_pixels(i64::from(i32::MAX), 18), -9);
        assert_eq!(reference_length_unit(10, 0, 18).unwrap(), 2);
        assert_eq!(reference_length_unit(-10, 0, 18).unwrap(), -2);
        assert_eq!(reference_length_unit(10, -1, 18).unwrap(), 10);
    }

    #[test]
    fn html_length_units_share_int32_wrap_and_original_sign_rounding() {
        for (pixels, units) in [
            (i64::from(i32::MAX), 1),
            (i64::from(i32::MIN), 0),
            (1_073_741_824, -119_304_646),
            (-1_073_741_826, 119_304_645),
            (10, 2),
            (-10, -2),
        ] {
            assert_eq!(reference_length_unit(pixels, 0, 18).unwrap(), units);
            assert_eq!(reference_length_unit(pixels, -7, 18).unwrap(), pixels);
        }
        assert_eq!(
            reference_length_unit(i64::from(i32::MAX) + 1, 1, 18)
                .unwrap_err()
                .kind,
            HtmlQueryErrorKind::ResourceLimit
        );
        assert_eq!(
            reference_length_unit(10, 0, 0).unwrap_err().kind,
            HtmlQueryErrorKind::InvalidMeasurement
        );
    }

    #[test]
    fn whole_expression_budget_cannot_reset_between_nested_plans() {
        let mut budget = QueryBudget::default();
        budget.charge(limits().maximum_work_bytes - 1, 0).unwrap();
        assert!(QueryPlan::substring("abc", 18, &mut budget).is_err());
        let mut budget = QueryBudget::default();
        budget.charge(0, limits().maximum_measurements).unwrap();
        assert!(budget.charge(0, 1).is_err());
    }
}
