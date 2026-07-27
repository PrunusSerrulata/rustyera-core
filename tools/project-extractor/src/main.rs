use std::process::ExitCode;

use project_extractor::{Command, USAGE, extract_cache, parse_arguments};

fn main() -> ExitCode {
    let current_directory = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("cannot determine current directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let command = match parse_arguments(std::env::args_os().skip(1), &current_directory) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let Command::Extract(options) = command else {
        println!("{USAGE}");
        println!("Extracts embedded project sources and binary assets.");
        return ExitCode::SUCCESS;
    };
    match extract_cache(&options) {
        Ok(summary) => {
            println!(
                "extracted {} project files to {} ({} binary assets)",
                summary.extracted_files,
                options.output.display(),
                summary.extracted_binary_assets
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("project extraction failed: {error}");
            ExitCode::FAILURE
        }
    }
}
