use fuel_core::schema::build_schema;
use std::{
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

// stored in the root of the workspace
const SCHEMA_URL: &'static str = "../assets/schema.sdl";

fn main() {
    let task = env::args().nth(1);

    match task.as_ref().map(|it| it.as_str()) {
        Some(cmd) if cmd == "build" || cmd == "b" => {
            if let Err(_) = cargo_build_and_dump_schema() {
                std::process::exit(-1);
            }
        }
        _ => {}
    }
}

/// ensures that latest schema is always committed
#[test]
fn is_latest_schema_committed() {
    let current_content = fs::read(SCHEMA_URL).unwrap();
    assert_eq!(current_content, build_schema().finish().sdl().as_bytes());
}

fn cargo_build_and_dump_schema() -> Result<(), Box<dyn std::error::Error>> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(cargo)
        .current_dir(project_root())
        .args(&["build"])
        .status()?;

    if !status.success() {
        Err("cargo build failed")?;
    }

    dump_schema()?;

    Ok(())
}

fn dump_schema() -> Result<(), Box<dyn std::error::Error>> {
    let assets = env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .map(|f| {
            let f = f.as_path().join(SCHEMA_URL);
            let dir = f.parent().expect("Failed to read assets dir");
            fs::create_dir_all(dir).expect("Failed to create assets dir");

            f
        })
        .expect("Failed to fetch assets path");

    File::create(&assets)
        .and_then(|mut f| {
            f.write_all(build_schema().finish().sdl().as_bytes())?;
            f.sync_all()
        })
        .expect("Failed to write SDL schema to temporary file");

    Ok(())
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}
