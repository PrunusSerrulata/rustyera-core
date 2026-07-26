use std::process::Command;

fn extractor() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rustyera-source-extractor"))
}

#[test]
fn help_describes_source_extraction_and_succeeds() {
    let output = extractor().arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("rustyera-source-extractor"));
    assert!(stdout.contains("embedded UTF-8 project sources"));
}

#[test]
fn missing_input_is_a_usage_error() {
    let output = extractor().output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Usage: rustyera-source-extractor")
    );
}
