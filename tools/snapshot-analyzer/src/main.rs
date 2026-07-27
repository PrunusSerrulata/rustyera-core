use std::process::ExitCode;

use snapshot_analyzer::{Command, USAGE, analyze_file, parse_arguments, render_json, render_text};

fn main() -> ExitCode {
    let command = match parse_arguments(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let Command::Analyze(options) = command else {
        println!("{USAGE}");
        println!("Inspects a complete RustyEra runtime snapshot.");
        return ExitCode::SUCCESS;
    };
    let inspection = match analyze_file(&options.input) {
        Ok(inspection) => inspection,
        Err(error) => {
            eprintln!("snapshot analysis failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let output = if options.json {
        render_json(&inspection)
    } else {
        render_text(&inspection)
    };
    match output {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("snapshot analysis failed: {error}");
            ExitCode::FAILURE
        }
    }
}
