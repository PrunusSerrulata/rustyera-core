use super::{
    HtmlLength, HtmlQueryError, HtmlStringLengthSettings, invalid_measurement, resource_limit,
};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// Reference WidthF/XsubPixel are float32. Store bits to retain owned Eq/serde
/// state without changing signed residuals into an exact-rational approximation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct Fraction {
    bits: u32,
}

impl Fraction {
    pub(super) const ZERO: Self = Self { bits: 0 };

    fn new(value: f32) -> Result<Self, HtmlQueryError> {
        if !value.is_finite() {
            return Err(invalid_measurement());
        }
        Ok(Self {
            bits: value.to_bits(),
        })
    }

    pub(super) fn add(self, other: Self) -> Result<Self, HtmlQueryError> {
        Self::new(f32::from_bits(self.bits) + f32::from_bits(other.bits))
    }

    pub(super) fn split(self) -> Result<(i64, Self), HtmlQueryError> {
        let value = f32::from_bits(self.bits);
        if !value.is_finite() {
            return Err(invalid_measurement());
        }
        let truncated = f64::from(value.trunc());
        if truncated < f64::from(i32::MIN) || truncated > f64::from(i32::MAX) {
            return Err(resource_limit());
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "reference truncation is intentional after the finite int32 bounds check"
        )]
        let pixels = value.trunc() as i64;
        #[expect(
            clippy::cast_precision_loss,
            reason = "reference XsubPixel subtraction deliberately uses float32"
        )]
        let residual = value - pixels as f32;
        Ok((pixels, Self::new(residual)?))
    }
}

pub(super) fn bounded_pixels(pixels: i64) -> Result<i64, HtmlQueryError> {
    i32::try_from(pixels)
        .map(i64::from)
        .map_err(|_| resource_limit())
}

pub(super) fn add_pixels(a: i64, b: i64) -> Result<i64, HtmlQueryError> {
    bounded_pixels(a.checked_add(b).ok_or_else(resource_limit)?)
}

pub(super) fn text_pixels(millipixels: i64) -> Result<i64, HtmlQueryError> {
    if millipixels < 0 {
        return Err(invalid_measurement());
    }
    // The snake StringMeasure returns an int per whole part/prefix, not after
    // summing fractional parts. Original GDI measurements are already integers;
    // browser font/backend differences are recorded outside this pure layout.
    bounded_pixels(millipixels / 1000)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "reference WidthF converts each operand to float32 before arithmetic"
)]
pub(super) fn length(value: HtmlLength, font_size: i32) -> Result<Fraction, HtmlQueryError> {
    match value {
        HtmlLength::Pixels(value) => Fraction::new(value as f32),
        HtmlLength::FontHeightHundredths(value) => {
            Fraction::new(value as f32 * font_size as f32 / 100.0)
        }
    }
}

pub(super) fn integer_length(value: HtmlLength, font_size: i32) -> Result<i64, HtmlQueryError> {
    bounded_pixels(match value {
        HtmlLength::Pixels(value) => i64::from(value),
        HtmlLength::FontHeightHundredths(value) => i64::from(value) * i64::from(font_size) / 100,
    })
}

pub(super) fn image_width(
    height: Option<HtmlLength>,
    width: Option<HtmlLength>,
    natural_width: u32,
    natural_height: u32,
    settings: HtmlStringLengthSettings,
) -> Result<(i64, Fraction), HtmlQueryError> {
    let nonzero = |value: HtmlLength| match value {
        HtmlLength::Pixels(value) | HtmlLength::FontHeightHundredths(value) => value != 0,
    };
    let height = height
        .filter(|value| nonzero(*value))
        .map(|value| integer_length(value, settings.font_size_pixels))
        .transpose()?
        .unwrap_or(i64::from(settings.font_size_pixels));
    let (width, residual) = if let Some(width) = width.filter(|value| nonzero(*value)) {
        let integer = integer_length(width, settings.font_size_pixels)?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "reference image residuals convert each operand to float32 before arithmetic"
        )]
        let fractional = match width {
            HtmlLength::Pixels(_) => 0.0,
            HtmlLength::FontHeightHundredths(value) => {
                settings.font_size_pixels as f32 * value as f32 / 100.0 - integer as f32
            }
        };
        (integer, Fraction::new(fractional)?)
    } else {
        if natural_height == 0 {
            return Err(invalid_measurement());
        }
        let integer = bounded_pixels(
            height
                .checked_mul(i64::from(natural_width))
                .ok_or_else(resource_limit)?
                / i64::from(natural_height),
        )?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "reference aspect-ratio residuals use float32 including the integer subtraction"
        )]
        let fractional =
            natural_width as f32 * height as f32 / natural_height as f32 - integer as f32;
        (integer, Fraction::new(fractional)?)
    };
    // The image constructor flips only the integer rectangle, not XsubPixel.
    Ok((
        bounded_pixels(width.checked_abs().ok_or_else(resource_limit)?)?,
        residual,
    ))
}

pub(super) fn image_fallback(
    source: &str,
    hover: Option<&str>,
    mask: Option<&str>,
    height: Option<HtmlLength>,
    width: Option<HtmlLength>,
    y: Option<HtmlLength>,
    font_size: i32,
) -> Result<String, HtmlQueryError> {
    let mut text = format!("<img src='{source}'");
    if let Some(hover) = hover {
        let _ = write!(text, " srcb='{hover}'");
    }
    if let Some(mask) = mask.filter(|mask| !mask.is_empty()) {
        let _ = write!(text, " srcm='{mask}'");
    }
    for (name, value) in [("height", height), ("width", width), ("ypos", y)] {
        if let Some(value) = value {
            let (raw, pixels) = match value {
                HtmlLength::Pixels(raw) => (raw, true),
                HtmlLength::FontHeightHundredths(raw) => (raw, false),
            };
            if raw != 0 {
                // MixedNum.BuilderString has this unusual conversion for non-px
                // input. This is reference AltText, not canonical HTML serialization.
                let number = if pixels {
                    i64::from(raw)
                } else {
                    integer_length(value, font_size)?
                };
                let _ = write!(text, " {name}='{number}{}'", if pixels { "px" } else { "" });
            }
        }
    }
    text.push('>');
    Ok(text)
}

pub(super) fn shape_advance(
    kind: &str,
    parameters: &[HtmlLength],
    font_size: i32,
) -> Result<Option<Fraction>, HtmlQueryError> {
    let raw = |value: HtmlLength| match value {
        HtmlLength::Pixels(value) | HtmlLength::FontHeightHundredths(value) => value,
    };
    match (kind.to_ascii_lowercase().as_str(), parameters) {
        ("space", [width]) => Ok(Some(length(*width, font_size)?)),
        ("rect", [width]) if raw(*width) > 0 => Ok(Some(length(*width, font_size)?)),
        ("rect", [x, _, width, height]) if raw(*x) >= 0 && raw(*width) > 0 && raw(*height) > 0 => {
            Ok(Some(
                length(*x, font_size)?.add(length(*width, font_size)?)?,
            ))
        }
        _ => Ok(None),
    }
}

pub(super) fn shape_fallback(
    kind: &str,
    parameters: &[HtmlLength],
    color: Option<u32>,
    button_color: Option<u32>,
    settings: HtmlStringLengthSettings,
) -> String {
    let parameters = parameters
        .iter()
        .map(|value| match value {
            HtmlLength::Pixels(value) => format!("{value}px"),
            HtmlLength::FontHeightHundredths(value) => value.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!(
        "<shape type='{}' param='{parameters}'",
        kind.to_ascii_lowercase()
    );
    if let Some(color) = color {
        let _ = write!(text, " color='#{color:06X}'");
    }
    if let Some(color) = button_color.filter(|color| *color != settings.focus_rgb) {
        let _ = write!(text, " bcolor='#{color:06X}'");
    }
    text.push('>');
    text
}
