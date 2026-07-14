use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourceIoError,
    SourceIoErrorKind, SourcePayload, analyze_project,
};
use erabasic_ast::{ParseOutput, SourceKind};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
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
                ":lex SOURCE       tokenize SOURCE\n:expr SOURCE      parse an expression\n:line SOURCE      parse a logical line\n:file PATH        parse a UTF-8 .ERH or .ERB file\n:analyze PATH...  analyze an ordered UTF-8 project source set as JSON\n:quit             exit"
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
        } else if let Some(paths) = input.strip_prefix(":analyze ") {
            analyze_files(paths.split_whitespace());
        } else {
            print_output(parse_line(input, &context));
        }
    }
    Ok(())
}

fn analyze_files<'a>(paths: impl IntoIterator<Item = &'a str>) {
    let project = load_project(&ProjectFiles::default(), &CsvLoadOptions::default());
    let Some(project_data) = project.data else {
        for diagnostic in project.diagnostics {
            eprintln!("{diagnostic:?}");
        }
        return;
    };
    let sources = paths
        .into_iter()
        .map(|path| ProjectSource {
            relative_path: path.replace('\\', "/"),
            payload: match fs::read_to_string(path) {
                Ok(text) => SourcePayload::Utf8(text),
                Err(error) => SourcePayload::IoError(SourceIoError {
                    kind: io_error_kind(error.kind()),
                    message: error.to_string(),
                }),
            },
        })
        .collect();
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources,
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("cannot serialize analysis result: {error}"),
    }
}

fn io_error_kind(kind: io::ErrorKind) -> SourceIoErrorKind {
    match kind {
        io::ErrorKind::NotFound => SourceIoErrorKind::NotFound,
        io::ErrorKind::PermissionDenied => SourceIoErrorKind::PermissionDenied,
        io::ErrorKind::InvalidData => SourceIoErrorKind::InvalidData,
        io::ErrorKind::Interrupted => SourceIoErrorKind::Interrupted,
        _ => SourceIoErrorKind::Other,
    }
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
