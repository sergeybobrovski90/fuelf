use std::process::Command;
use std::path::PathBuf;

pub fn generate_abi(contract: &str) {
    let base = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut abi_path = PathBuf::from(base);

    let out = std::env::var("OUT_DIR").unwrap();
    let mut out_dir = PathBuf::from(out);

    abi_path.push(contract);
    out_dir.push("abi_code.rs");

    Command::new("fuels-abi-cli")
        .arg("codegen")
        .arg("no_name")
        .arg(abi_path.to_str().unwrap())
        .arg(out_dir.to_str().unwrap())
        .arg("--no-std")
        .status()
        .expect("Failed to generate ABI.");
}
