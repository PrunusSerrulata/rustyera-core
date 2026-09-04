// Derived from ICU 72.1 Unicode 15.0 exports and Unicode 15.0 IndicSyllabicCategory.
// Unicode license: see source manifest. Fixed data, independent of the Rust toolchain.
mod gcb_high_bmp;
mod gcb_low;
mod gcb_supplementary;
mod properties;

pub(super) use gcb_high_bmp::GCB_HIGH_BMP;
pub(super) use gcb_low::GCB_LOW;
pub(super) use gcb_supplementary::GCB_SUPPLEMENTARY;
pub(super) use properties::{EXTENDED_PICTOGRAPHIC, LINKING_CONSONANT, NONZERO_CCC, VIRAMA};
