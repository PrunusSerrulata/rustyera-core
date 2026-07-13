use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use erabasic_ast::{ParseOutput, SourceKind};
use erabasic_lexer::lex;
use erabasic_parser::{
    DefaultParserContext, ParserContext, parse_erb, parse_erh, parse_expression, parse_line,
};

fn main() -> io::Result<()> {
    let mut context = DefaultParserContext::default();
    println!("RustyEra parser REPL. Type :help for commands.");
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let input = line.trim_end_matches(['\r', '\n']);
        if input.is_empty() {
            continue;
        }
        if input == ":quit" || input == ":q" {
            break;
        }
        if input == ":help" {
            println!(
                ":lex SOURCE  tokenize SOURCE\n:expr SOURCE parse an expression\n:line SOURCE parse a logical line\n:file PATH   parse a UTF-8 .ERH or .ERB file\n:quit        exit"
            );
            continue;
        }
        if let Some(source) = input.strip_prefix(":lex ") {
            let output = lex(source, context.lexer_config());
            println!("{:#?}", output.tokens);
            print_diagnostics(&output.diagnostics);
        } else if let Some(source) = input.strip_prefix(":expr ") {
            print_output(parse_expression(source, &context));
        } else if let Some(source) = input.strip_prefix(":line ") {
            print_output(parse_line(source, &context));
        } else if let Some(path) = input.strip_prefix(":file ") {
            match fs::read_to_string(path) {
                Ok(source) => {
                    let kind = if Path::new(path)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("erh"))
                    {
                        SourceKind::Erh
                    } else {
                        SourceKind::Erb
                    };
                    if kind == SourceKind::Erh {
                        print_output(parse_erh(&source, &mut context));
                    } else {
                        print_output(parse_erb(&source, &mut context));
                    }
                }
                Err(error) => eprintln!("cannot read {path:?}: {error}"),
            }
        } else {
            print_output(parse_line(input, &context));
        }
    }
    Ok(())
}

fn print_output<T: std::fmt::Debug>(output: ParseOutput<T>) {
    if let Some(value) = output.value {
        println!("{value:#?}");
    }
    print_diagnostics(&output.diagnostics);
}

fn print_diagnostics(diagnostics: &[erabasic_ast::Diagnostic]) {
    for diagnostic in diagnostics {
        eprintln!("{diagnostic}");
    }
}
