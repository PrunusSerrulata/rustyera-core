//! Character CSV numeric parsing and diagnostic construction.

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, input::ascii_fold,
    reader::EnabledLine, tables::at_line,
};

pub(super) fn parse_era_integer(value: &str) -> Option<i64> {
    let (value, sign) = if let Some(rest) = value.strip_prefix('+') {
        (rest, 1)
    } else if let Some(rest) = value.strip_prefix('-') {
        (rest, -1)
    } else {
        (value, 1)
    };
    let (digits, radix) = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        (
            hex.chars()
                .take_while(char::is_ascii_hexdigit)
                .collect::<String>(),
            16,
        )
    } else if let Some(binary) = value
        .strip_prefix("0b")
        .or_else(|| value.strip_prefix("0B"))
    {
        (
            binary
                .chars()
                .take_while(|character| matches!(character, '0' | '1'))
                .collect::<String>(),
            2,
        )
    } else {
        (
            value
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>(),
            10,
        )
    };
    (!digits.is_empty())
        .then(|| {
            i64::from_str_radix(&digits, radix)
                .ok()
                .map(|number| number * sign)
        })
        .flatten()
}

pub(super) fn character_csv_number(path: &str) -> i64 {
    let upper = ascii_fold(path);
    let Some(start) = upper.find("CHARA") else {
        return 0;
    };
    path[start + "CHARA".len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

pub(super) fn equal_keyword(left: &str, right: &str, ignore_case: bool) -> bool {
    if ignore_case {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

pub(super) fn duplicate_field(
    variable: &str,
    index: usize,
    line: &EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    diagnostics.push(at_line(
        CsvDiagnosticCode::DuplicateCharacterField,
        CsvDiagnosticSeverity::Warning,
        1,
        line,
        format!("{variable}:{index} is defined more than once; the last value wins"),
    ));
}

pub(super) fn index_out_of_range(text: &str, line: &EnabledLine) -> CsvDiagnostic {
    at_line(
        CsvDiagnosticCode::IndexOutOfRange,
        CsvDiagnosticSeverity::Warning,
        1,
        line,
        format!("character array index {text:?} is out of range"),
    )
}
