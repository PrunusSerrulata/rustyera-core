use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::super::HtmlElementKind;
use super::{
    HtmlDecodedSource, HtmlOutputOrigin, HtmlOutputPiece, HtmlQueryEntityPolicy, HtmlQueryError,
    HtmlQueryErrorKind, HtmlQueryLimits, HtmlQueryProbe, HtmlQueryProbeKind, HtmlSourceRange,
    HtmlSubstringResult, decode_query_entities, parse_document_with_source_map,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlSubstringPoll {
    NeedMeasure(HtmlQueryProbe),
    Complete(HtmlSubstringResult),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct OpenTag {
    range: HtmlSourceRange,
    name: String,
    style: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct AwaitingMeasure {
    probe: HtmlQueryProbe,
    next_cursor: usize,
}

/// Pull-based substring evaluator. A caller measures exactly the requested canonical document.
/// Pixel widths are integer `HtmlLength` results, already quantized by the runtime's policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlSubstringPlan {
    decoded: HtmlDecodedSource,
    limits: HtmlQueryLimits,
    remaining_pixels: i64,
    cursor: usize,
    open: Vec<OpenTag>,
    content: bool,
    next_probe: u64,
    measurements: usize,
    work_bytes: usize,
    awaiting: Option<AwaitingMeasure>,
    completed: Option<HtmlSubstringResult>,
}

impl HtmlSubstringPlan {
    /// Whole-unescape now, but defer markup parsing/measurement until the scan reaches it.
    ///
    /// # Errors
    /// Reports whole-input entity/Unicode errors or exceeded source limits.
    pub fn new(
        source: &str,
        pixel_budget: i64,
        limits: HtmlQueryLimits,
    ) -> Result<Self, HtmlQueryError> {
        if source.len() > limits.maximum_work_bytes {
            return Err(work_limit());
        }
        let decoded = decode_query_entities(source, HtmlQueryEntityPolicy::ReferenceQuery, limits)?;
        Ok(Self {
            decoded,
            limits,
            remaining_pixels: pixel_budget,
            cursor: 0,
            open: Vec::new(),
            content: false,
            next_probe: 1,
            measurements: 0,
            work_bytes: source.len(),
            awaiting: None,
            completed: None,
        })
    }

    /// Cumulative work and measurements, for a runtime's whole-expression budget.
    #[must_use]
    pub const fn usage(&self) -> (usize, usize) {
        (self.work_bytes, self.measurements)
    }

    /// The working-source map is relative to the original input of this one split invocation.
    #[must_use]
    pub const fn decoded_source(&self) -> &HtmlDecodedSource {
        &self.decoded
    }

    /// Return one needed measurement or the completed source-preserving result.
    /// Calling poll repeatedly while a probe is pending does not advance the evaluator.
    ///
    /// # Errors
    /// Reports only errors at the currently reached scan frontier, or query limits.
    pub fn poll(&mut self) -> Result<HtmlSubstringPoll, HtmlQueryError> {
        self.validate_state()?;
        if let Some(completed) = &self.completed {
            return Ok(HtmlSubstringPoll::Complete(completed.clone()));
        }
        if let Some(awaiting) = &self.awaiting {
            return Ok(HtmlSubstringPoll::NeedMeasure(awaiting.probe.clone()));
        }
        while self.cursor < self.decoded.text.len() {
            if self.decoded.text[self.cursor..].starts_with('<') {
                if let Some(probe) = self.scan_tag()? {
                    return Ok(HtmlSubstringPoll::NeedMeasure(probe));
                }
                if let Some(completed) = &self.completed {
                    return Ok(HtmlSubstringPoll::Complete(completed.clone()));
                }
            } else {
                let character = self.decoded.text[self.cursor..]
                    .chars()
                    .next()
                    .ok_or_else(invalid_split_state)?;
                let end = self.cursor + character.len_utf8();
                let mut fragment = String::new();
                for tag in self.open.iter().rev().filter(|tag| tag.style) {
                    append_bounded(
                        &mut fragment,
                        &self.decoded.text[tag.range.start..tag.range.end],
                        self.limits.maximum_output_bytes,
                    )?;
                }
                append_bounded(
                    &mut fragment,
                    &self.decoded.text[self.cursor..end],
                    self.limits.maximum_output_bytes,
                )?;
                for tag in self.open.iter().filter(|tag| tag.style) {
                    append_bounded(
                        &mut fragment,
                        &format!("</{}>", tag.name),
                        self.limits.maximum_output_bytes,
                    )?;
                }
                let probe = self.prepare_probe(&fragment, HtmlQueryProbeKind::Scalar, end)?;
                return Ok(HtmlSubstringPoll::NeedMeasure(probe));
            }
        }
        self.finish(self.cursor, self.cursor)?;
        Ok(HtmlSubstringPoll::Complete(
            self.completed.clone().ok_or_else(invalid_split_state)?,
        ))
    }

    /// Complete only the currently requested probe. Invalid/old IDs leave that probe pending.
    ///
    /// # Errors
    /// Rejects unsolicited IDs, negative scalar widths, arithmetic overflow, and output limits.
    pub fn resume(
        &mut self,
        probe_id: u64,
        integer_pixel_width: i64,
    ) -> Result<(), HtmlQueryError> {
        self.validate_state()?;
        let awaiting = self
            .awaiting
            .as_ref()
            .filter(|pending| pending.probe.id == probe_id)
            .ok_or_else(|| {
                HtmlQueryError::new(
                    HtmlQueryErrorKind::InvalidMeasurement,
                    self.cursor,
                    self.cursor,
                    "measurement does not match the pending probe",
                )
            })?;
        if awaiting.probe.kind == HtmlQueryProbeKind::Scalar && integer_pixel_width < 0 {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::InvalidMeasurement,
                self.cursor,
                awaiting.next_cursor,
                "scalar width must not be negative",
            ));
        }
        let remaining = self
            .remaining_pixels
            .checked_sub(integer_pixel_width)
            .ok_or_else(|| {
                HtmlQueryError::new(
                    HtmlQueryErrorKind::InvalidMeasurement,
                    self.cursor,
                    awaiting.next_cursor,
                    "pixel budget subtraction overflow",
                )
            })?;
        let next_cursor = awaiting.next_cursor;
        let scalar = awaiting.probe.kind == HtmlQueryProbeKind::Scalar;
        self.awaiting = None;
        if remaining < 0 && (scalar || self.content) {
            return self.finish(self.cursor, self.cursor);
        }
        self.remaining_pixels = remaining;
        self.cursor = next_cursor;
        // Deliberately not set for image/shape: reference's content flag is text-specific.
        self.content |= scalar;
        Ok(())
    }

    /// Validate addressable owned state after deserialization; this does not authorize restoring
    /// an external VM wait or certify the source identity. The runtime still owns those checks.
    ///
    /// # Errors
    /// Rejects invalid scalar offsets, malformed pending probes and exceeded plan limits.
    pub fn validate_state(&self) -> Result<(), HtmlQueryError> {
        let legal = |offset| {
            offset <= self.decoded.text.len() && self.decoded.text.is_char_boundary(offset)
        };
        if !legal(self.cursor)
            || self.open.len() > self.limits.maximum_depth
            || self.decoded.text.len() > self.limits.maximum_source_bytes
            || self.decoded.boundaries.len().saturating_sub(1) > self.limits.maximum_scalars
            || self.work_bytes > self.limits.maximum_work_bytes
            || self.measurements > self.limits.maximum_measurements
            || self.open.iter().any(|tag| {
                tag.range.start > tag.range.end || !legal(tag.range.start) || !legal(tag.range.end)
            })
            || self.awaiting.as_ref().is_some_and(|pending| {
                !legal(pending.next_cursor)
                    || pending.next_cursor <= self.cursor
                    || pending.probe.id >= self.next_probe
                    || pending.probe.source.start != self.cursor
                    || pending.probe.source.end != pending.next_cursor
            })
            || self.completed.as_ref().is_some_and(|result| {
                result.head.len() > self.limits.maximum_output_bytes
                    || result.tail.len() > self.limits.maximum_output_bytes
            })
        {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::InvalidMarkup,
                0,
                0,
                "invalid owned HTML query state",
            ));
        }
        if let Some(awaiting) = &self.awaiting {
            super::check_document(&awaiting.probe.document, self.limits)?;
        }
        Ok(())
    }

    fn prepare_probe(
        &mut self,
        fragment: &str,
        kind: HtmlQueryProbeKind,
        end: usize,
    ) -> Result<HtmlQueryProbe, HtmlQueryError> {
        if self.measurements >= self.limits.maximum_measurements {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::ResourceLimit,
                self.cursor,
                end,
                "HTML measurement limit exceeded",
            ));
        }
        self.consume_work(fragment.len())?;
        let document = parse_document_with_source_map(
            fragment,
            HtmlQueryEntityPolicy::ReferenceQuery,
            self.limits,
        )
        .map_err(|error| {
            HtmlQueryError::new(
                error.kind,
                self.cursor,
                end,
                &format!("measurement fragment: {}", error.message),
            )
        })?
        .document;
        let probe = HtmlQueryProbe {
            id: self.next_probe,
            kind,
            document,
            source: HtmlSourceRange {
                start: self.cursor,
                end,
            },
        };
        self.next_probe = self.next_probe.checked_add(1).ok_or_else(|| {
            HtmlQueryError::new(
                HtmlQueryErrorKind::ResourceLimit,
                self.cursor,
                end,
                "probe identity exhausted",
            )
        })?;
        self.measurements += 1;
        self.awaiting = Some(AwaitingMeasure {
            probe: probe.clone(),
            next_cursor: end,
        });
        Ok(probe)
    }

    fn scan_tag(&mut self) -> Result<Option<HtmlQueryProbe>, HtmlQueryError> {
        let start = self.cursor;
        // SUBSTRING's lexical scanner uses the first '>', unlike the ordinary quoted-aware
        // parser. Actual probes still pass through that one canonical semantic parser.
        let Some(end) = self.decoded.text[start + 1..]
            .find('>')
            .map(|at| start + 1 + at + 1)
        else {
            self.finish(start, start)?;
            return Ok(None);
        };
        self.consume_work(end - start)?;
        let raw = &self.decoded.text[start + 1..end - 1];
        if let Some(closing) = raw.strip_prefix('/') {
            let name = closing.trim();
            if HtmlElementKind::parse(name).is_none() {
                return Err(HtmlQueryError::new(
                    HtmlQueryErrorKind::UnsupportedTag,
                    start,
                    end,
                    "closing tag is outside the existing dialect",
                ));
            }
            self.open.pop().ok_or_else(|| {
                HtmlQueryError::new(
                    HtmlQueryErrorKind::InvalidMarkup,
                    start,
                    end,
                    "closing tag has no opening scope",
                )
            })?;
            self.cursor = end;
            return Ok(None);
        }
        let name = raw.split(' ').next().unwrap_or(raw).to_owned();
        let canonical_name = raw
            .trim()
            .trim_end_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("");
        let comment = self.decoded.text[start..].starts_with("<!--");
        if !comment && HtmlElementKind::parse(canonical_name).is_none() {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::UnsupportedTag,
                start,
                end,
                "tag is outside the existing dialect",
            ));
        }
        if name == "br" {
            // Preserve the historical fixed four-byte removal, including its noncanonical
            // variants. For exact <br> this is the complete break. All offsets remain UTF-8 safe.
            let tail = start
                .checked_add(4)
                .filter(|tail| {
                    *tail <= self.decoded.text.len() && self.decoded.text.is_char_boundary(*tail)
                })
                .ok_or_else(|| {
                    HtmlQueryError::new(
                        HtmlQueryErrorKind::InvalidMarkup,
                        start,
                        end,
                        "break removal is not a legal scalar boundary",
                    )
                })?;
            self.finish(start, tail)?;
            return Ok(None);
        }
        if matches!(name.as_str(), "img" | "shape") {
            let fragment = self.decoded.text[start..end].to_owned();
            return self
                .prepare_probe(&fragment, HtmlQueryProbeKind::Atomic, end)
                .map(Some);
        }
        if self.open.len() >= self.limits.maximum_depth {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::ResourceLimit,
                start,
                end,
                "HTML query scope depth exceeded",
            ));
        }
        self.open.push(OpenTag {
            range: HtmlSourceRange { start, end },
            style: matches!(name.as_str(), "b" | "i" | "s"),
            name,
        });
        self.cursor = end;
        Ok(None)
    }

    fn consume_work(&mut self, bytes: usize) -> Result<(), HtmlQueryError> {
        self.work_bytes = self
            .work_bytes
            .checked_add(bytes)
            .filter(|bytes| *bytes <= self.limits.maximum_work_bytes)
            .ok_or_else(work_limit)?;
        Ok(())
    }

    fn finish(&mut self, cut: usize, tail_start: usize) -> Result<(), HtmlQueryError> {
        if !self.decoded.text.is_char_boundary(cut)
            || !self.decoded.text.is_char_boundary(tail_start)
        {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::InvalidUnicode,
                cut,
                tail_start,
                "substring cut is not a scalar boundary",
            ));
        }
        let mut head = String::new();
        let mut tail = String::new();
        let mut head_pieces = Vec::new();
        let mut tail_pieces = Vec::new();
        append_piece(
            &mut head,
            &mut head_pieces,
            &self.decoded.text[..cut],
            HtmlOutputOrigin::Working(HtmlSourceRange { start: 0, end: cut }),
            self.limits,
        )?;
        for tag in self.open.iter().rev() {
            append_piece(
                &mut head,
                &mut head_pieces,
                &format!("</{}>", tag.name),
                HtmlOutputOrigin::GeneratedClose { opening: tag.range },
                self.limits,
            )?;
        }
        for tag in &self.open {
            append_piece(
                &mut tail,
                &mut tail_pieces,
                &self.decoded.text[tag.range.start..tag.range.end],
                HtmlOutputOrigin::Reopened { opening: tag.range },
                self.limits,
            )?;
        }
        append_piece(
            &mut tail,
            &mut tail_pieces,
            &self.decoded.text[tail_start..],
            HtmlOutputOrigin::Working(HtmlSourceRange {
                start: tail_start,
                end: self.decoded.text.len(),
            }),
            self.limits,
        )?;
        self.completed = Some(HtmlSubstringResult {
            head,
            tail,
            head_pieces,
            tail_pieces,
            consumed_working_bytes: tail_start,
        });
        Ok(())
    }
}

fn invalid_split_state() -> HtmlQueryError {
    HtmlQueryError::new(
        HtmlQueryErrorKind::InvalidMeasurement,
        0,
        0,
        "invalid substring evaluator state",
    )
}

fn append_bounded(output: &mut String, text: &str, limit: usize) -> Result<(), HtmlQueryError> {
    if output.len().saturating_add(text.len()) > limit {
        return Err(HtmlQueryError::new(
            HtmlQueryErrorKind::ResourceLimit,
            0,
            0,
            "HTML query output exceeds its limit",
        ));
    }
    output.push_str(text);
    Ok(())
}

fn work_limit() -> HtmlQueryError {
    HtmlQueryError::new(
        HtmlQueryErrorKind::ResourceLimit,
        0,
        0,
        "HTML cumulative source/measurement work exceeds its limit",
    )
}

fn append_piece(
    output: &mut String,
    pieces: &mut Vec<HtmlOutputPiece>,
    text: &str,
    origin: HtmlOutputOrigin,
    limits: HtmlQueryLimits,
) -> Result<(), HtmlQueryError> {
    let start = output.len();
    append_bounded(output, text, limits.maximum_output_bytes)?;
    if !text.is_empty() {
        pieces.push(HtmlOutputPiece {
            output: HtmlSourceRange {
                start,
                end: output.len(),
            },
            origin,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlLinesPoll {
    NeedMeasure(HtmlQueryProbe),
    Complete(u64),
}

/// Repeated pure split evaluation, with no runtime variable writes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlStringLinesPlan {
    current: String,
    split: Option<HtmlSubstringPlan>,
    pixel_budget: i64,
    limits: HtmlQueryLimits,
    count: u64,
    measurements: usize,
    work_bytes: usize,
    seen: BTreeSet<[u8; 32]>,
}

impl HtmlStringLinesPlan {
    /// Start a line count. Empty input completes immediately without requesting measurement.
    ///
    /// # Errors
    /// Reports initial entity/Unicode/source-limit failures for nonempty input.
    pub fn new(
        source: &str,
        pixel_budget: i64,
        limits: HtmlQueryLimits,
    ) -> Result<Self, HtmlQueryError> {
        super::check_source(source, limits)?;
        let split = if source.is_empty() {
            None
        } else {
            Some(HtmlSubstringPlan::new(source, pixel_budget, limits)?)
        };
        Ok(Self {
            current: source.into(),
            split,
            pixel_budget,
            limits,
            count: 0,
            measurements: 0,
            work_bytes: source.len(),
            seen: BTreeSet::from([*blake3::hash(source.as_bytes()).as_bytes()]),
        })
    }

    /// Current per-line input, including synthesized reopenings. After the first line this is
    /// not the original script string; diagnostics must not mislabel its byte spans as original.
    #[must_use]
    pub fn current_source(&self) -> &str {
        &self.current
    }

    /// Current working-source map. Its source coordinates refer to `current_source()`.
    #[must_use]
    pub fn decoded_source(&self) -> Option<&HtmlDecodedSource> {
        self.split.as_ref().map(HtmlSubstringPlan::decoded_source)
    }

    /// Validate structural owned state only; the runtime must separately authorize VM wait restore.
    ///
    /// # Errors
    /// Rejects invalid counters, excessive retained history, or malformed inner split state.
    pub fn validate_state(&self) -> Result<(), HtmlQueryError> {
        if self.current.len() > self.limits.maximum_source_bytes
            || self.work_bytes > self.limits.maximum_work_bytes
            || self.measurements > self.limits.maximum_measurements
            || self.count > self.limits.maximum_lines as u64
            || self.seen.len() > self.limits.maximum_lines.saturating_add(1)
        {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::InvalidMarkup,
                0,
                0,
                "invalid owned HTML line query state",
            ));
        }
        if let Some(split) = &self.split {
            split.validate_state()?;
        }
        Ok(())
    }

    /// Advance through zero-measurement transitions or request the next real measurement.
    ///
    /// # Errors
    /// Returns no-progress/cycle, per-query limits, or the reached split/decoder failure.
    pub fn poll(&mut self) -> Result<HtmlLinesPoll, HtmlQueryError> {
        self.validate_state()?;
        loop {
            let Some(split) = &mut self.split else {
                return Ok(HtmlLinesPoll::Complete(self.count));
            };
            if self.count >= self.limits.maximum_lines as u64 {
                return Err(HtmlQueryError::new(
                    HtmlQueryErrorKind::ResourceLimit,
                    0,
                    self.current.len(),
                    "HTML line count limit exceeded",
                ));
            }
            split.limits.maximum_work_bytes = split.work_bytes.saturating_add(
                self.limits
                    .maximum_work_bytes
                    .saturating_sub(self.work_bytes),
            );
            split.limits.maximum_measurements = split.measurements.saturating_add(
                self.limits
                    .maximum_measurements
                    .saturating_sub(self.measurements),
            );
            let before_work = split.work_bytes;
            let poll = split.poll()?;
            self.work_bytes = self
                .work_bytes
                .checked_add(split.work_bytes - before_work)
                .filter(|bytes| *bytes <= self.limits.maximum_work_bytes)
                .ok_or_else(work_limit)?;
            match poll {
                HtmlSubstringPoll::NeedMeasure(probe) => {
                    return Ok(HtmlLinesPoll::NeedMeasure(probe));
                }
                HtmlSubstringPoll::Complete(result) => {
                    let next_probe = split.next_probe;
                    if result.tail.is_empty() {
                        self.count += 1;
                        self.current.clear();
                        self.split = None;
                        return Ok(HtmlLinesPoll::Complete(self.count));
                    }
                    if result.tail == self.current {
                        return Err(HtmlQueryError::new(
                            HtmlQueryErrorKind::NoProgress,
                            0,
                            self.current.len(),
                            "HTML split did not advance",
                        ));
                    }
                    self.work_bytes = self
                        .work_bytes
                        .checked_add(result.tail.len())
                        .filter(|bytes| *bytes <= self.limits.maximum_work_bytes)
                        .ok_or_else(work_limit)?;
                    let digest = *blake3::hash(result.tail.as_bytes()).as_bytes();
                    if !self.seen.insert(digest) {
                        return Err(HtmlQueryError::new(
                            HtmlQueryErrorKind::NoProgress,
                            0,
                            self.current.len(),
                            "HTML split did not advance or repeated an earlier tail",
                        ));
                    }
                    self.count += 1;
                    self.current = result.tail;
                    self.work_bytes = self
                        .work_bytes
                        .checked_add(self.current.len())
                        .filter(|bytes| *bytes <= self.limits.maximum_work_bytes)
                        .ok_or_else(work_limit)?;
                    // Re-run whole Unescape for every tail, including its reopened tags.
                    let mut next =
                        HtmlSubstringPlan::new(&self.current, self.pixel_budget, self.limits)?;
                    next.next_probe = next_probe;
                    self.split = Some(next);
                }
            }
        }
    }

    /// Complete the outstanding probe without exposing substring side effects.
    ///
    /// # Errors
    /// Rejects stale IDs, invalid measurements and total work beyond the whole-query budget.
    pub fn resume(
        &mut self,
        probe_id: u64,
        integer_pixel_width: i64,
    ) -> Result<(), HtmlQueryError> {
        self.validate_state()?;
        if self.measurements >= self.limits.maximum_measurements {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::ResourceLimit,
                0,
                0,
                "HTML line measurement limit exceeded",
            ));
        }
        let split = self.split.as_mut().ok_or_else(|| {
            HtmlQueryError::new(
                HtmlQueryErrorKind::InvalidMeasurement,
                0,
                0,
                "line query has already completed",
            )
        })?;
        split.resume(probe_id, integer_pixel_width)?;
        self.measurements += 1;
        Ok(())
    }
}
