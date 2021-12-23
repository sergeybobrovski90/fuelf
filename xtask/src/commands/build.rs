use super::dump::dump_schema;
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct BuildCommand {}

pub fn cargo_build_and_dump_schema() -> Result<(), Box<dyn std::error::Error>> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(cargo)
        .current_dir(project_root())
        .args(&["build"])
        .status()?;

    if !status.success() {
        return Err("cargo build failed".into());
    }

    dump_schema()?;

    Ok(())
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}
