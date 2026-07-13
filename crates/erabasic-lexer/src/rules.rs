use crate::Operator;

pub(crate) fn is_identifier_start(ch: char) -> bool {
    !is_identifier_delimiter(ch)
}

pub(crate) fn is_identifier_delimiter(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\u{3000}'
            | '\t'
            | '\r'
            | '\n'
            | '.'
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '='
            | '!'
            | '<'
            | '>'
            | '|'
            | '&'
            | '^'
            | '~'
            | '?'
            | '#'
            | ')'
            | '}'
            | ']'
            | ','
            | ':'
            | '('
            | '{'
            | '['
            | '$'
            | '\\'
            | '\''
            | '"'
            | '@'
            | ';'
    )
}

pub(crate) fn operator_at(source: &str) -> Option<(Operator, usize)> {
    const OPS: &[(&str, Operator)] = &[
        ("<<=", Operator::ShiftLeftAssign),
        (">>=", Operator::ShiftRightAssign),
        ("&&", Operator::LogicalAnd),
        ("||", Operator::LogicalOr),
        ("^^", Operator::LogicalXor),
        ("!&", Operator::Nand),
        ("!|", Operator::Nor),
        ("==", Operator::Equal),
        ("!=", Operator::NotEqual),
        ("<=", Operator::LessEqual),
        (">=", Operator::GreaterEqual),
        ("<<", Operator::ShiftLeft),
        (">>", Operator::ShiftRight),
        ("++", Operator::Increment),
        ("--", Operator::Decrement),
        ("+=", Operator::AddAssign),
        ("-=", Operator::SubtractAssign),
        ("*=", Operator::MultiplyAssign),
        ("/=", Operator::DivideAssign),
        ("%=", Operator::ModuloAssign),
        ("&=", Operator::BitAndAssign),
        ("|=", Operator::BitOrAssign),
        ("^=", Operator::BitXorAssign),
        ("=", Operator::Assign),
        ("<", Operator::Less),
        (">", Operator::Greater),
        ("+", Operator::Add),
        ("-", Operator::Subtract),
        ("*", Operator::Multiply),
        ("/", Operator::Divide),
        ("%", Operator::Modulo),
        ("&", Operator::BitAnd),
        ("|", Operator::BitOr),
        ("^", Operator::BitXor),
        ("~", Operator::BitNot),
        ("!", Operator::LogicalNot),
        ("?", Operator::Question),
        ("#", Operator::TernarySeparator),
    ];
    OPS.iter()
        .find_map(|(text, op)| source.starts_with(text).then_some((*op, text.len())))
}
