use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema");
    fs::create_dir_all(&output_directory)?;
    fs::write(
        output_directory.join("reraconfig.schema.json"),
        era_config::generate_json_schema(),
    )?;
    fs::write(
        output_directory.join("reraconfig.example.toml"),
        era_config::generate_annotated_example(),
    )?;
    Ok(())
}
