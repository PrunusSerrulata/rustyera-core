/// Produce the canonical lookup key used by project-defined `EraBasic` identifiers.
///
/// `EraBasic`'s compatibility folding is deliberately ASCII-only: non-ASCII names
/// retain their exact spelling even when case-insensitive lookup is enabled.
pub(crate) fn identifier_key(name: &str, ignore_case: bool) -> String {
    if ignore_case {
        name.to_ascii_uppercase()
    } else {
        name.to_owned()
    }
}

pub(crate) fn is_reserved(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "IS" | "TO"
            | "INT"
            | "STR"
            | "REFFUNC"
            | "STATIC"
            | "DYNAMIC"
            | "GLOBAL"
            | "PRIVATE"
            | "SAVEDATA"
            | "CHARADATA"
            | "REF"
            | "__DEBUG__"
            | "__SKIP__"
            | "_"
    )
}

#[cfg(test)]
mod tests {
    use super::identifier_key;

    #[test]
    fn identifier_keys_preserve_case_policy_and_non_ascii_spelling() {
        assert_eq!(identifier_key("Target", true), "TARGET");
        assert_eq!(identifier_key("Target", false), "Target");
        assert_eq!(identifier_key("変数名", true), "変数名");
    }
}
