#[path = "ordinal_casing_data/bmp_high.rs"]
mod bmp_high;
#[path = "ordinal_casing_data/bmp_low.rs"]
mod bmp_low;
#[path = "ordinal_casing_data/latin.rs"]
mod latin;
#[path = "ordinal_casing_data/supplementary.rs"]
mod supplementary;

pub(super) use bmp_high::ICU72_BMP_SIMPLE_UPPER_HIGH;
pub(super) use bmp_low::ICU72_BMP_SIMPLE_UPPER_LOW;
pub(super) use latin::{LATIN_UPPER, NO_CASING_PAGES};
pub(super) use supplementary::DOTNET_SUPPLEMENTARY;
