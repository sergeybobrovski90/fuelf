use assert_cmd::prelude::*;
use fuel_core::service::{
    Config,
    FuelService,
};
// Add methods on commands
use fuel_core_e2e_client::{
    config::SuiteConfig,
    CONFIG_FILE_KEY,
};
use std::{
    fs,
    io::Write,
    process::Command,
};
use tempfile::TempDir; // Used for writing assertions // Run programs

#[tokio::test(flavor = "multi_thread")]
async fn works_in_local_env() -> Result<(), Box<dyn std::error::Error>> {
    // setup a local node
    let srv = setup_local_node().await;
    // generate a config file
    let config = generate_config_file(srv.bound_address.to_string());

    let mut cmd = Command::cargo_bin("fuel-core-e2e-client")?;
    let cmd = cmd.env(CONFIG_FILE_KEY, config.path).assert().success();
    std::io::stdout()
        .write_all(&cmd.get_output().stdout)
        .unwrap();
    std::io::stderr()
        .write_all(&cmd.get_output().stderr)
        .unwrap();
    Ok(())
}

async fn setup_local_node() -> FuelService {
    FuelService::new_node(Config::local_node()).await.unwrap()
}

fn generate_config_file(endpoint: String) -> TestConfig {
    // generate a tmp dir
    let tmp_dir = TempDir::new().unwrap();
    // setup config for test env
    let config = SuiteConfig {
        endpoint,
        ..Default::default()
    };
    // write config to file
    let config_path = tmp_dir.path().join("config.toml");
    fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();

    TestConfig {
        path: config_path.to_str().unwrap().to_string(),
        _dir: tmp_dir,
    }
}

struct TestConfig {
    path: String,
    // keep the temp dir alive to defer the deletion of the temp dir until the end of the test
    _dir: TempDir,
}
