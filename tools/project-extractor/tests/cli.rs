use std::process::Command;

fn extractor() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rustyera-project-extractor"))
}

#[test]
fn help_describes_project_extraction_and_succeeds() {
    let output = extractor().arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("rustyera-project-extractor"));
    assert!(stdout.contains("project sources and binary assets"));
}

#[test]
fn missing_input_is_a_usage_error() {
    let output = extractor().output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Usage: rustyera-project-extractor")
    );
}
