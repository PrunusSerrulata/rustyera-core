//! Portable named-color normalization shared by `EraBasic` and presentation code.

/// Resolve the platform-independent named-color subset used by bundled scripts.
///
/// The fixed reference delegates to `System.Drawing.Color.FromName`. System
/// colors are client-specific, so this table intentionally contains ordinary
/// RGB colors only. Unknown names and `Transparent` return `None`.
#[must_use]
pub fn named_color(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "black" => 0x0000_0000,
        "white" => 0x00ff_ffff,
        "red" => 0x00ff_0000,
        "green" => 0x0000_8000,
        "blue" => 0x0000_00ff,
        "yellow" => 0x00ff_ff00,
        "gray" | "grey" => 0x0080_8080,
        "dimgray" | "dimgrey" => 0x0069_6969,
        "silver" => 0x00c0_c0c0,
        "maroon" => 0x0080_0000,
        "purple" => 0x0080_0080,
        "fuchsia" | "magenta" => 0x00ff_00ff,
        "lime" => 0x0000_ff00,
        "olive" => 0x0080_8000,
        "navy" => 0x0000_0080,
        "teal" => 0x0000_8080,
        "aqua" | "cyan" => 0x0000_ffff,
        "orange" => 0x00ff_a500,
        "pink" => 0x00ff_c0cb,
        "brown" => 0x00a5_2a2a,
        "gold" => 0x00ff_d700,
        "lightsalmon" => 0x00ff_a07a,
        "darkseagreen" => 0x008f_bc8f,
        "darkred" => 0x008b_0000,
        "deepskyblue" => 0x0000_bfff,
        "lightgreen" => 0x0090_ee90,
        "royalblue" => 0x0041_69e1,
        "skyblue" => 0x0087_ceeb,
        "salmon" => 0x00fa_8072,
        "blanchedalmond" => 0x00ff_ebcd,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_script_colors_case_insensitively_without_system_colors() {
        assert_eq!(named_color("LightSalmon"), Some(0x00ff_a07a));
        assert_eq!(named_color("DEEPSKYBLUE"), Some(0x0000_bfff));
        assert_eq!(named_color("Magenta"), Some(0x00ff_00ff));
        assert_eq!(named_color("dimgray"), Some(0x0069_6969));
        assert_eq!(named_color("transparent"), None);
        assert_eq!(named_color("ControlText"), None);
    }
}
