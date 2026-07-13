use erabasic_parser::{DefaultParserContext, parse_erb, parse_erh, parse_expression, parse_line};

#[test]
fn crate_root_keeps_all_parser_entry_points() {
    let mut context = DefaultParserContext::default();
    assert!(parse_expression("1 + 2", &context).value.is_some());
    assert!(parse_line("RESULT = 1", &context).value.is_some());
    assert!(parse_erh("#DEFINE ONE 1\n", &mut context).value.is_some());
    assert!(parse_erb("@TEST\nRETURN\n", &mut context).value.is_some());
}
