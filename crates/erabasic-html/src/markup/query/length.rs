//! Owned `HTML_STRINGLEN` planning. Providers measure parts, never choose lines.

mod build;
mod geometry;
mod layout;
mod suffix;

use super::super::{HtmlAlignment, HtmlDocument, HtmlLength};
use super::{
    HtmlMappedDocument, HtmlQueryEntityPolicy, HtmlQueryError, HtmlQueryErrorKind, HtmlQueryLimits,
    HtmlSourceRange, parse_document_with_source_map,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Values come from the query base configuration, not the script's current bold/font state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlStringLengthSettings {
    pub font_size_pixels: i32,
    pub drawable_width_pixels: i32,
    pub prevent_button_wrap: bool,
    pub legacy_nonbutton_wrap: bool,
    pub foreground_rgb: u32,
    pub focus_rgb: u32,
}

/// UTF offsets describe one scalar boundary. Raw-string plans also retain the original
/// source byte. Canonical subprobes have no original entity lexeme and report None.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlLengthCut {
    pub decoded_utf8: usize,
    pub decoded_utf16: usize,
    pub source_byte: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlLengthProbeKind {
    /// Prefixes must be shaped independently, without the unselected suffix.
    TextPart {
        text_node_path: Vec<usize>,
        cuts: Vec<HtmlLengthCut>,
    },
    /// Resolve the existing sprite and, only when absent, measure this plain-text `AltText`.
    ImageSlot { missing_document: HtmlDocument },
    /// A reference error-shape's `AltText` is visible but cannot be divided.
    FallbackText,
    /// Shape/div geometry is core-owned. The renderer still validates readiness/errors.
    FixedSlot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlLengthProbe {
    pub id: u64,
    pub document: HtmlDocument,
    pub kind: HtmlLengthProbeKind,
    pub source: HtmlSourceRange,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlLengthImageResolution {
    Loaded {
        natural_width: u32,
        natural_height: u32,
    },
    Missing {
        fallback_advance_millipixels: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlLengthMeasuredValue {
    /// Exactly one value per requested cut, including zero and the full string.
    TextPart {
        prefix_advances_millipixels: Vec<i64>,
    },
    ImageSlot(HtmlLengthImageResolution),
    FallbackText {
        advance_millipixels: i64,
    },
    FixedSlotReady,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlLengthMeasurement {
    pub probe_id: u64,
    pub value: HtmlLengthMeasuredValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlStringLengthResult {
    pub first_line_pixels: i64,
    pub value: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlStringLengthPoll {
    NeedMeasurements { probe_ids: Vec<u64> },
    Complete(HtmlStringLengthResult),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlStringLengthPlan {
    settings: HtmlStringLengthSettings,
    pixel_flag: i64,
    limits: HtmlQueryLimits,
    source: LengthSource,
    probes: Vec<HtmlLengthProbe>,
    values: Vec<Option<HtmlLengthMeasuredValue>>,
    parts: Vec<Part>,
    layouts: Vec<Layout>,
    root_layout: usize,
    suffix_probes: BTreeMap<usize, BTreeMap<usize, usize>>,
    work_bytes: usize,
    measurement_units: usize,
    completed: Option<HtmlStringLengthResult>,
    failure: Option<HtmlQueryError>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum LengthSource {
    Mapped(HtmlMappedDocument),
    Canonical(HtmlDocument),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Part {
    probe: usize,
    kind: PartKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum PartKind {
    Text,
    Shape {
        advance: geometry::Fraction,
    },
    Fallback {
        utf16_length: usize,
    },
    Image {
        height: Option<HtmlLength>,
        width: Option<HtmlLength>,
        fallback_utf16_length: usize,
    },
    Division {
        absolute: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Button {
    parts: Vec<usize>,
    clickable: bool,
    position: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Entry {
    Button(Button),
    Break,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Layout {
    entries: Vec<Entry>,
    no_break: bool,
    alignment: HtmlAlignment,
    width: i64,
}

/// Convert the completed first-row width using the reference HTML-only units.
/// The reference doubles an unchecked int32, then rounds in the direction of
/// the original width's sign, even when doubling has wrapped to the other sign.
/// A nonzero flag preserves the checked int32 pixel width unchanged.
///
/// # Errors
/// Rejects widths outside the existing int32 geometry bound and a nonpositive
/// default font size. This does not relax checked layout accumulation.
pub fn html_string_length_units(
    pixels: i64,
    flag: i64,
    font_size: i32,
) -> Result<i64, HtmlQueryError> {
    let pixels = i32::try_from(pixels).map_err(|_| resource_limit())?;
    if flag != 0 {
        return Ok(i64::from(pixels));
    }
    if font_size <= 0 {
        return Err(error(
            HtmlQueryErrorKind::InvalidMeasurement,
            "invalid default font size",
        ));
    }
    let doubled = pixels.wrapping_mul(2);
    let correction = if doubled % font_size == 0 {
        0
    } else if pixels >= 0 {
        1
    } else {
        -1
    };
    Ok(i64::from(doubled / font_size + correction))
}

impl HtmlStringLengthPlan {
    /// Parse the entire argument and plan every part before emitting any probe.
    /// This differs intentionally from the lazy `HTML_SUBSTRING` scanner.
    ///
    /// # Errors
    /// Rejects malformed markup, reference-invalid positions/scopes and exceeded limits.
    pub fn new(
        source: &str,
        settings: HtmlStringLengthSettings,
        pixel_flag: i64,
        limits: HtmlQueryLimits,
    ) -> Result<Self, HtmlQueryError> {
        if settings.font_size_pixels <= 0
            || settings.drawable_width_pixels <= 0
            || settings.foreground_rgb > 0x00ff_ffff
            || settings.focus_rgb > 0x00ff_ffff
        {
            return Err(error(
                HtmlQueryErrorKind::InvalidMeasurement,
                "invalid HTML query base settings",
            ));
        }
        let mapped =
            parse_document_with_source_map(source, HtmlQueryEntityPolicy::ReferenceQuery, limits)?;
        let built = build::build(source, &mapped, settings, limits)?;
        let values = vec![None; built.probes.len()];
        Ok(Self {
            settings,
            pixel_flag,
            limits,
            source: LengthSource::Mapped(mapped),
            probes: built.probes,
            values,
            parts: built.parts,
            layouts: built.layouts,
            root_layout: built.root_layout,
            suffix_probes: BTreeMap::new(),
            work_bytes: built.work_bytes,
            measurement_units: built.measurement_units,
            completed: None,
            failure: None,
        })
    }

    /// Use an already parsed scalar/atomic `HTML_SUBSTRING` subprobe without serializing
    /// or parsing it again. Canonical Text LF is an actual newline in this independent
    /// `HtmlLength` input; only the raw-string constructor can distinguish entity LF.
    ///
    /// # Errors
    /// Rejects invalid settings, excessive trees and incompatible semantic structure.
    pub fn from_document(
        document: HtmlDocument,
        settings: HtmlStringLengthSettings,
        pixel_flag: i64,
        limits: HtmlQueryLimits,
    ) -> Result<Self, HtmlQueryError> {
        if settings.font_size_pixels <= 0
            || settings.drawable_width_pixels <= 0
            || settings.foreground_rgb > 0x00ff_ffff
            || settings.focus_rgb > 0x00ff_ffff
        {
            return Err(invalid_measurement());
        }
        super::check_document(&document, limits)?;
        let built = build::build_document(&document, settings, limits)?;
        let values = vec![None; built.probes.len()];
        Ok(Self {
            settings,
            pixel_flag,
            limits,
            source: LengthSource::Canonical(document),
            probes: built.probes,
            values,
            parts: built.parts,
            layouts: built.layouts,
            root_layout: built.root_layout,
            suffix_probes: BTreeMap::new(),
            work_bytes: built.work_bytes,
            measurement_units: built.measurement_units,
            completed: None,
            failure: None,
        })
    }

    #[must_use]
    pub fn probes(&self) -> &[HtmlLengthProbe] {
        &self.probes
    }

    /// Cumulative (work bytes, measurement units), including pure layout replays
    /// and dynamically appended suffixes. Nested runtime adapters account deltas
    /// against the outer operation; creating a child does not reset that budget.
    #[must_use]
    pub fn usage(&self) -> (usize, usize) {
        (self.work_bytes, self.measurement_units)
    }

    #[must_use]
    pub fn mapped_document(&self) -> Option<&HtmlMappedDocument> {
        match &self.source {
            LengthSource::Mapped(mapped) => Some(mapped),
            LengthSource::Canonical(_) => None,
        }
    }

    #[must_use]
    pub fn document(&self) -> &HtmlDocument {
        match &self.source {
            LengthSource::Mapped(mapped) => &mapped.document,
            LengthSource::Canonical(document) => document,
        }
    }

    #[must_use]
    pub fn has_measurement(&self, probe_id: u64) -> bool {
        usize::try_from(probe_id)
            .ok()
            .and_then(|index| self.values.get(index))
            .is_some_and(Option::is_some)
    }

    /// Record one typed reply. A failed provider probe must terminate the external
    /// operation; callers must not omit it and return an already measured first row.
    ///
    /// # Errors
    /// Rejects unknown/duplicate IDs, missing cuts, invalid widths and wrong variants.
    pub fn resume(&mut self, measurement: HtmlLengthMeasurement) -> Result<(), HtmlQueryError> {
        if self.completed.is_some() || self.failure.is_some() {
            return Err(invalid_measurement());
        }
        let index = usize::try_from(measurement.probe_id).map_err(|_| invalid_measurement())?;
        let probe = self.probes.get(index).ok_or_else(invalid_measurement)?;
        if probe.id != measurement.probe_id || self.values.get(index).is_none_or(Option::is_some) {
            return Err(invalid_measurement());
        }
        validate_value(probe, &measurement.value)?;
        self.values[index] = Some(measurement.value);
        Ok(())
    }

    /// Drive the complete layout, requesting independently shaped suffixes as needed.
    /// Initial probes all complete before layout; the first row is not published until
    /// every later row and division child has completed without error.
    ///
    /// # Errors
    /// Rejects invalid layout state and arithmetic/work limits. Layout failures are sticky.
    pub fn poll(&mut self) -> Result<HtmlStringLengthPoll, HtmlQueryError> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        if let Some(result) = self.completed {
            return Ok(HtmlStringLengthPoll::Complete(result));
        }
        match self.poll_inner() {
            Ok(result) => Ok(result),
            Err(error) => {
                self.failure = Some(error.clone());
                Err(error)
            }
        }
    }

    fn poll_inner(&mut self) -> Result<HtmlStringLengthPoll, HtmlQueryError> {
        if self.probes.len() != self.values.len() {
            return Err(invalid_measurement());
        }
        let pending = self.pending_probe_ids();
        if !pending.is_empty() {
            return Ok(HtmlStringLengthPoll::NeedMeasurements { probe_ids: pending });
        }
        self.charge_work(
            self.probes.len().saturating_mul(64).saturating_add(
                self.measurement_units
                    .saturating_mul(std::mem::size_of::<i64>()),
            ),
        )?;
        self.validate_state()?;
        let mut first_line_pixels = 0;
        let mut lines = 0_usize;
        // Pure layout replay consumes the same bounded work budget as new suffixes.
        // It retains no mutable renderer or borrowed frame state across service waits.
        for index in 0..self.layouts.len() {
            let Some(result) = layout::complete(self, index)? else {
                return Ok(HtmlStringLengthPoll::NeedMeasurements {
                    probe_ids: self.pending_probe_ids(),
                });
            };
            lines = lines.checked_add(result.lines).ok_or_else(resource_limit)?;
            if lines > self.limits.maximum_lines {
                return Err(resource_limit());
            }
            let pixels = result.first_line_pixels;
            if index == self.root_layout {
                first_line_pixels = pixels;
            }
        }
        let value = html_string_length_units(
            first_line_pixels,
            self.pixel_flag,
            self.settings.font_size_pixels,
        )?;
        let result = HtmlStringLengthResult {
            first_line_pixels,
            value,
        };
        self.completed = Some(result);
        Ok(HtmlStringLengthPoll::Complete(result))
    }

    /// Convenience for callers that already supplied every required measurement.
    /// Service adapters must use poll/resume, including newly appended suffix probes.
    ///
    /// # Errors
    /// Returns `InvalidMeasurement` while more probes are needed, or the actual layout error.
    pub fn finish(&mut self) -> Result<HtmlStringLengthResult, HtmlQueryError> {
        match self.poll()? {
            HtmlStringLengthPoll::Complete(result) => Ok(result),
            HtmlStringLengthPoll::NeedMeasurements { .. } => Err(invalid_measurement()),
        }
    }

    fn pending_probe_ids(&self) -> Vec<u64> {
        self.values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| value.is_none().then_some(index as u64))
            .collect()
    }

    fn charge_work(&mut self, bytes: usize) -> Result<(), HtmlQueryError> {
        self.work_bytes = self.work_bytes.saturating_add(bytes);
        if self.work_bytes > self.limits.maximum_work_bytes {
            return Err(resource_limit());
        }
        Ok(())
    }

    /// Structural defense only: this does not authorize restoring an external wait.
    /// Runtime still owns snapshot/lifecycle/projection validation.
    ///
    /// # Errors
    /// Rejects inconsistent indices, collection limits and malformed recorded values.
    pub fn validate_state(&self) -> Result<(), HtmlQueryError> {
        if self.probes.len() != self.values.len()
            || self.probes.len() > self.limits.maximum_measurements
            || self.parts.len() > self.limits.maximum_nodes
            || self.layouts.len() > self.limits.maximum_nodes
            || self.root_layout >= self.layouts.len()
            || self.settings.font_size_pixels <= 0
            || self.work_bytes > self.limits.maximum_work_bytes
            || self.measurement_units > self.limits.maximum_measurements
        {
            return Err(invalid_measurement());
        }
        let mut cuts = 0_usize;
        for (index, probe) in self.probes.iter().enumerate() {
            if probe.id != index as u64 {
                return Err(invalid_measurement());
            }
            if let HtmlLengthProbeKind::TextPart {
                cuts: boundaries, ..
            } = &probe.kind
            {
                cuts = cuts.saturating_add(boundaries.len());
            }
            if let Some(value) = &self.values[index] {
                validate_value(probe, value)?;
            }
        }
        if cuts > self.limits.maximum_measurements {
            return Err(resource_limit());
        }
        for (parent, suffixes) in &self.suffix_probes {
            for (cut, probe) in suffixes {
                if *cut == 0 || *parent >= *probe || *probe >= self.probes.len() {
                    return Err(invalid_measurement());
                }
            }
        }
        for part in &self.parts {
            if part.probe >= self.probes.len() {
                return Err(invalid_measurement());
            }
        }
        for layout in &self.layouts {
            for entry in &layout.entries {
                if let Entry::Button(button) = entry
                    && (button.parts.is_empty()
                        || button.parts.iter().any(|part| *part >= self.parts.len()))
                {
                    return Err(invalid_measurement());
                }
            }
        }
        Ok(())
    }
}

fn validate_value(
    probe: &HtmlLengthProbe,
    value: &HtmlLengthMeasuredValue,
) -> Result<(), HtmlQueryError> {
    match (&probe.kind, value) {
        (
            HtmlLengthProbeKind::TextPart { cuts, .. },
            HtmlLengthMeasuredValue::TextPart {
                prefix_advances_millipixels,
            },
        ) => {
            if prefix_advances_millipixels.len() != cuts.len()
                || prefix_advances_millipixels.first() != Some(&0)
            {
                return Err(invalid_measurement());
            }
            for width in prefix_advances_millipixels {
                geometry::text_pixels(*width)?;
            }
        }
        (HtmlLengthProbeKind::ImageSlot { .. }, HtmlLengthMeasuredValue::ImageSlot(resolution)) => {
            match resolution {
                HtmlLengthImageResolution::Loaded {
                    natural_width,
                    natural_height,
                } => {
                    if *natural_width == 0
                        || *natural_height == 0
                        || *natural_width > 1_048_576
                        || *natural_height > 1_048_576
                    {
                        return Err(invalid_measurement());
                    }
                }
                HtmlLengthImageResolution::Missing {
                    fallback_advance_millipixels,
                } => {
                    geometry::text_pixels(*fallback_advance_millipixels)?;
                }
            }
        }
        (
            HtmlLengthProbeKind::FallbackText,
            HtmlLengthMeasuredValue::FallbackText {
                advance_millipixels,
            },
        ) => {
            geometry::text_pixels(*advance_millipixels)?;
        }
        (HtmlLengthProbeKind::FixedSlot, HtmlLengthMeasuredValue::FixedSlotReady) => {}
        _ => return Err(invalid_measurement()),
    }
    Ok(())
}

fn error(kind: HtmlQueryErrorKind, message: &str) -> HtmlQueryError {
    HtmlQueryError::new(kind, 0, 0, message)
}
fn input_error(kind: HtmlQueryErrorKind, message: &str) -> HtmlQueryError {
    HtmlQueryError::input(kind, 0, 0, message)
}
fn invalid_measurement() -> HtmlQueryError {
    error(
        HtmlQueryErrorKind::InvalidMeasurement,
        "invalid or incomplete HTML length measurements",
    )
}
fn resource_limit() -> HtmlQueryError {
    error(
        HtmlQueryErrorKind::ResourceLimit,
        "HTML length layout exceeds its bounded arithmetic/work budget",
    )
}

#[cfg(test)]
mod tests;
