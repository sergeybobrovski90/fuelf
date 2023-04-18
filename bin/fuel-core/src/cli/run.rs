#![allow(unused_variables)]
use crate::{
    cli::{
        init_logging,
        run::consensus::PoATriggerArgs,
        DEFAULT_DB_PATH,
    },
    FuelService,
};
use anyhow::{
    anyhow,
    Context,
};
use clap::Parser;
use fuel_core::{
    chain_config::{
        default_consensus_dev_key,
        ChainConfig,
    },
    producer::Config as ProducerConfig,
    service::{
        config::Trigger,
        Config,
        DbType,
        RelayerVerifierConfig,
        ServiceTrait,
        VMConfig,
    },
    txpool::Config as TxPoolConfig,
    types::{
        blockchain::primitives::SecretKeyWrapper,
        fuel_tx::Address,
        fuel_vm::SecretKey,
        secrecy::{
            ExposeSecret,
            Secret,
        },
    },
};
use pyroscope::PyroscopeAgent;
use pyroscope_pprofrs::{
    pprof_backend,
    PprofConfig,
};
use std::{
    env,
    net,
    ops::Deref,
    path::PathBuf,
    str::FromStr,
};
use tracing::{
    info,
    log::warn,
    trace,
};

pub const CONSENSUS_KEY_ENV: &str = "CONSENSUS_KEY_SECRET";

#[cfg(feature = "p2p")]
mod p2p;

mod consensus;
mod profiling;
#[cfg(feature = "relayer")]
mod relayer;

/// Run the Fuel client node locally.
#[derive(Debug, Clone, Parser)]
pub struct Command {
    #[clap(long = "ip", default_value = "127.0.0.1", value_parser, env)]
    pub ip: net::IpAddr,

    #[clap(long = "port", default_value = "4000", env)]
    pub port: u16,

    /// Vanity name for node, used in telemetry
    #[clap(long = "service-name", default_value = "fuel-core", value_parser, env)]
    pub service_name: String,

    #[clap(
        name = "DB_PATH",
        long = "db-path",
        value_parser,
        default_value = (*DEFAULT_DB_PATH).to_str().unwrap(),
        env
    )]
    pub database_path: PathBuf,

    #[clap(
        long = "db-type",
        default_value = "rocks-db",
        value_enum,
        ignore_case = true,
        env
    )]
    pub database_type: DbType,

    /// Specify either an alias to a built-in configuration or filepath to a JSON file.
    #[arg(
        name = "CHAIN_CONFIG",
        long = "chain",
        default_value = "local_testnet",
        env
    )]
    pub chain_config: String,

    /// Allows GraphQL Endpoints to arbitrarily advanced blocks. Should be used for local development only
    #[arg(long = "manual_blocks_enabled", env)]
    pub manual_blocks_enabled: bool,

    /// Enable logging of backtraces from vm errors
    #[arg(long = "vm-backtrace", env)]
    pub vm_backtrace: bool,

    /// Enable full utxo stateful validation
    /// disabled by default until downstream consumers stabilize
    #[arg(long = "utxo-validation", env)]
    pub utxo_validation: bool,

    /// The minimum allowed gas price
    #[arg(long = "min-gas-price", default_value = "0", env)]
    pub min_gas_price: u64,

    /// The signing key used when producing blocks.
    /// Setting via the `CONSENSUS_KEY_SECRET` ENV var is preferred.
    #[arg(long = "consensus-key", env)]
    pub consensus_key: Option<String>,

    /// A new block is produced instantly when transactions are available.
    #[clap(flatten)]
    pub poa_trigger: PoATriggerArgs,

    /// Use a default insecure consensus key for testing purposes.
    /// This will not be enabled by default in the future.
    #[arg(long = "dev-keys", default_value = "true", env)]
    pub consensus_dev_key: bool,

    /// The block's fee recipient public key.
    ///
    /// If not set, `consensus_key` is used as the provider of the `Address`.
    #[arg(long = "coinbase-recipient", env)]
    pub coinbase_recipient: Option<String>,

    #[cfg(feature = "relayer")]
    #[clap(flatten)]
    pub relayer_args: relayer::RelayerArgs,

    #[cfg_attr(feature = "p2p", clap(flatten))]
    #[cfg(feature = "p2p")]
    pub p2p_args: p2p::P2PArgs,

    #[cfg_attr(feature = "p2p", clap(flatten))]
    #[cfg(feature = "p2p")]
    pub sync_args: p2p::SyncArgs,

    #[arg(long = "metrics", env)]
    pub metrics: bool,

    #[clap(long = "verify_max_da_lag", default_value = "10", env)]
    pub max_da_lag: u64,
    #[clap(long = "verify_max_relayer_wait", default_value = "30s", env)]
    pub max_wait_time: humantime::Duration,

    #[clap(long = "tx-pool-ttl", default_value = "5m", env)]
    pub tx_pool_ttl: humantime::Duration,

    #[clap(long = "honeycomb-api-key", env)]
    pub honeycomb_key: Option<String>,

    #[clap(flatten)]
    pub profiling: profiling::ProfilingArgs,
}

impl Command {
    pub fn get_config(self) -> anyhow::Result<Config> {
        let Command {
            ip,
            port,
            service_name: name,
            database_path,
            database_type,
            chain_config,
            vm_backtrace,
            manual_blocks_enabled,
            utxo_validation,
            min_gas_price,
            consensus_key,
            poa_trigger,
            consensus_dev_key,
            coinbase_recipient,
            #[cfg(feature = "relayer")]
            relayer_args,
            #[cfg(feature = "p2p")]
            p2p_args,
            #[cfg(feature = "p2p")]
            sync_args,
            metrics,
            max_da_lag,
            max_wait_time,
            tx_pool_ttl,
            honeycomb_key,
            profiling: _,
        } = self;

        let addr = net::SocketAddr::new(ip, port);

        let chain_conf: ChainConfig = chain_config.as_str().parse()?;

        #[cfg(feature = "p2p")]
        let p2p_cfg = p2p_args.into_config(metrics)?;

        let trigger: Trigger = poa_trigger.into();

        if trigger != Trigger::Never {
            info!("Block production mode: {:?}", &trigger);
        } else {
            info!("Block production disabled");
        }

        // if consensus key is not configured, fallback to dev consensus key
        let consensus_key = load_consensus_key(consensus_key)?.or_else(|| {
            if consensus_dev_key && trigger != Trigger::Never {
                let key = default_consensus_dev_key();
                warn!(
                    "Fuel Core is using an insecure test key for consensus. Public key: {}",
                    key.public_key()
                );
                Some(Secret::new(key.into()))
            } else {
                // if consensus dev key is disabled, use no key
                None
            }
        });

        if consensus_key.is_some() && trigger == Trigger::Never {
            warn!("Consensus key configured but block production is disabled!")
        }

        let coinbase_recipient = if let Some(coinbase_recipient) = coinbase_recipient {
            Address::from_str(coinbase_recipient.as_str()).map_err(|err| anyhow!(err))?
        } else {
            consensus_key
                .as_ref()
                .cloned()
                .map(|key| {
                    let sk = key.expose_secret().deref();
                    Address::from(*sk.public_key().hash())
                })
                .unwrap_or_default()
        };

        let verifier = RelayerVerifierConfig {
            max_da_lag: max_da_lag.into(),
            max_wait_time: max_wait_time.into(),
        };

        Ok(Config {
            addr,
            database_path,
            database_type,
            chain_conf: chain_conf.clone(),
            utxo_validation,
            manual_blocks_enabled,
            block_production: trigger,
            vm: VMConfig {
                backtrace: vm_backtrace,
            },
            txpool: TxPoolConfig::new(
                chain_conf,
                min_gas_price,
                utxo_validation,
                metrics,
                tx_pool_ttl.into(),
            ),
            block_producer: ProducerConfig {
                utxo_validation,
                coinbase_recipient,
                metrics,
            },
            block_executor: Default::default(),
            block_importer: Default::default(),
            #[cfg(feature = "relayer")]
            relayer: relayer_args.into(),
            #[cfg(feature = "p2p")]
            p2p: p2p_cfg,
            #[cfg(feature = "p2p")]
            sync: sync_args.into(),
            consensus_key,
            name,
            verifier,
            honeycomb_api_key: honeycomb_key,
        })
    }
}

pub async fn exec(command: Command) -> anyhow::Result<()> {
    let profiling = command.profiling.clone();
    let config = command.get_config()?;
    let network_name = {
        #[cfg(feature = "p2p")]
        {
            config
                .p2p
                .as_ref()
                .map(|config| config.network_name.clone())
                .unwrap_or_else(|| "default_network".to_string())
        }
        #[cfg(not(feature = "p2p"))]
        "default_network".to_string()
    };
    init_logging(
        config.name.clone(),
        network_name.clone(),
        config.honeycomb_api_key.clone(),
    )
    .await?;

    // start profiling agent if url is configured
    let _profiling_agent = profiling
        .pyroscope_url
        .as_ref()
        .map(|url| -> anyhow::Result<_> {
            // Configure profiling backend
            let agent = PyroscopeAgent::builder(url, &"fuel-core".to_string())
                .tags(vec![
                    ("service", config.name.as_str()),
                    ("network", network_name.as_str()),
                ])
                .backend(pprof_backend(
                    PprofConfig::new().sample_rate(profiling.pprof_sample_rate),
                ))
                .build()
                .context("failed to start profiler")?;
            let agent_running = agent.start().unwrap();
            Ok(agent_running)
        })
        .transpose()?;

    // log fuel-core version
    info!("Fuel Core version v{}", env!("CARGO_PKG_VERSION"));
    trace!("Initializing in TRACE mode.");
    // initialize the server
    let server = FuelService::new_node(config).await?;
    // pause the main task while service is running
    tokio::select! {
        result = server.await_stop() => {
            result?;
        }
        _ = shutdown_signal() => {}
    }

    server.stop_and_await().await?;

    Ok(())
}

// Attempt to load the consensus key from cli arg first, otherwise check the env.
fn load_consensus_key(
    cli_arg: Option<String>,
) -> anyhow::Result<Option<Secret<SecretKeyWrapper>>> {
    let secret_string = if let Some(cli_arg) = cli_arg {
        warn!("Consensus key configured insecurely using cli args. Consider setting the {} env var instead.", CONSENSUS_KEY_ENV);
        Some(cli_arg)
    } else {
        env::var(CONSENSUS_KEY_ENV).ok()
    };

    if let Some(key) = secret_string {
        let key =
            SecretKey::from_str(&key).context("failed to parse consensus signing key")?;
        Ok(Some(Secret::new(key.into())))
    } else {
        Ok(None)
    }
}

async fn shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

        let mut sigint =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        loop {
            tokio::select! {
                _ = sigterm.recv() => {
                    tracing::info!("sigterm received");
                    break;
                }
                _ = sigint.recv() => {
                    tracing::log::info!("sigint received");
                    break;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        tracing::log::info!("CTRL+C received");
    }
    Ok(())
}
