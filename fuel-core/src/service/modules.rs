#![allow(clippy::let_unit_value)]
use crate::{
    database::Database,
    service::Config,
};
use anyhow::Result;
#[cfg(feature = "p2p")]
use fuel_core_interfaces::p2p::P2pDb;
#[cfg(feature = "relayer")]
use fuel_core_interfaces::relayer::RelayerDb;
use fuel_core_interfaces::{
    block_producer::BlockProducer,
    model::{
        BlockHeight,
        FuelBlock,
    },
    txpool::{
      Sender,
      TxPoolDb,
     }
};
use futures::future::join_all;
use std::sync::Arc;
use tokio::{
    sync::{
        broadcast,
        mpsc,
    },
    task::JoinHandle,
};
use tracing::info;

pub struct Modules {
    pub txpool: Arc<fuel_txpool::Service>,
    pub block_importer: Arc<fuel_block_importer::Service>,
    pub block_producer: Arc<dyn BlockProducer>,
    pub bft: Arc<fuel_core_bft::Service>,
    pub sync: Arc<fuel_sync::Service>,
    #[cfg(feature = "relayer")]
    pub relayer: Arc<fuel_relayer::Service>,
    #[cfg(feature = "p2p")]
    pub network_service: Arc<fuel_p2p::orchestrator::Service>,
}

impl Modules {
    pub async fn stop(&self) {
        let stops: Vec<JoinHandle<()>> = vec![
            self.txpool.stop().await,
            self.block_importer.stop().await,
            self.bft.stop().await,
            self.sync.stop().await,
            #[cfg(feature = "p2p")]
            self.network_service.stop().await,
        ]
        .into_iter()
        .flatten()
        .collect();

        join_all(stops).await;
    }
}

pub async fn start_modules(config: &Config, database: &Database) -> Result<Modules> {
    let db = ();
    // Initialize and bind all components
    let block_importer =
        fuel_block_importer::Service::new(&config.block_importer, db).await?;
    let block_producer = Arc::new(DummyBlockProducer);
    let bft = fuel_core_bft::Service::new(&config.bft, db).await?;
    let sync = fuel_sync::Service::new(&config.sync).await?;

    #[cfg(feature = "relayer")]
    let relayer = {
        let mut relayer_builder = fuel_relayer::ServiceBuilder::new();
        relayer_builder
            .config(config.relayer.clone())
            .db(Box::new(database.clone()) as Box<dyn RelayerDb>)
            .import_block_event(block_importer.subscribe())
            .private_key(
                hex::decode(
                    "c6bd905dcac2a0b1c43f574ab6933df14d7ceee0194902bce523ed054e8e798b",
                )
                .unwrap(),
            );

        relayer_builder.build()?
    };

    let relayer_sender = {
        #[cfg(feature = "relayer")]
        {
            relayer.sender().clone()
        }
        #[cfg(not(feature = "relayer"))]
        {
            fuel_core_interfaces::relayer::Sender::noop()
        }
    };

    let (tx_status_sender, mut tx_status_receiver) = broadcast::channel(100);

    // Remove once tx_status events are used
    tokio::spawn(async move { while (tx_status_receiver.recv().await).is_ok() {} });

    let (txpool_sender, txpool_receiver) = mpsc::channel(100);
    let (incoming_tx_sender, incoming_tx_receiver) = broadcast::channel(100);

    #[cfg(feature = "p2p")]
    let (p2p_request_event_sender, p2p_request_event_receiver) = mpsc::channel(100);
    #[cfg(feature = "p2p")]
    let (block_event_sender, block_event_receiver) = mpsc::channel(100);

    #[cfg(not(feature = "p2p"))]
    let (p2p_request_event_sender, _p2p_request_event_receiver) = mpsc::channel(100);
    #[cfg(not(feature = "p2p"))]
    let (_block_event_sender, block_event_receiver) = mpsc::channel(100);

    #[cfg(feature = "p2p")]
    let network_service = {
        let p2p_db: Arc<dyn P2pDb> = Arc::new(database.clone());
        let (tx_consensus, _) = mpsc::channel(100);
        fuel_p2p::orchestrator::Service::new(
            config.p2p.clone(),
            p2p_db,
            p2p_request_event_sender.clone(),
            p2p_request_event_receiver,
            tx_consensus,
            incoming_tx_sender,
            block_event_sender,
        )
    };
    #[cfg(not(feature = "p2p"))]
    {
        let keep_alive = Box::new(incoming_tx_sender);
        Box::leak(keep_alive);
    }

    let mut txpool_builder = fuel_txpool::ServiceBuilder::new();
    txpool_builder
        .config(config.txpool.clone())
        .db(Box::new(database.clone()) as Box<dyn TxPoolDb>)
        .incoming_tx_receiver(incoming_tx_receiver)
        .import_block_event(block_importer.subscribe())
        .tx_status_sender(tx_status_sender)
        .txpool_sender(Sender::new(txpool_sender))
        .txpool_receiver(txpool_receiver);

    #[cfg(feature = "p2p")]
    txpool_builder.network_sender(p2p_request_event_sender.clone());

    let txpool = txpool_builder.build()?;

    // start services

    block_importer.start().await;
    block_producer.start(txpool.sender().clone()).await;

    bft.start(
        relayer_sender.clone(),
        p2p_request_event_sender.clone(),
        block_producer.clone(),
        block_importer.sender().clone(),
        block_importer.subscribe(),
    )
    .await;

    sync.start(
        block_event_receiver,
        p2p_request_event_sender.clone(),
        relayer_sender,
        bft.sender().clone(),
        block_importer.sender().clone(),
    )
    .await;

    #[cfg(feature = "relayer")]
    if config.relayer.eth_client.is_some() {
        relayer.start().await?;
    }

    #[cfg(feature = "p2p")]
    if !config.p2p.network_name.is_empty() {
        network_service.start().await?;
    }

    txpool.start().await?;

    Ok(Modules {
        txpool: Arc::new(txpool),
        block_importer: Arc::new(block_importer),
        block_producer,
        bft: Arc::new(bft),
        sync: Arc::new(sync),
        #[cfg(feature = "relayer")]
        relayer: Arc::new(relayer),
        #[cfg(feature = "p2p")]
        network_service: Arc::new(network_service),
    })
}

// TODO: replace this with the real block producer
struct DummyBlockProducer;

#[async_trait::async_trait]
impl BlockProducer for DummyBlockProducer {
    async fn produce_block(&self, height: BlockHeight) -> Result<FuelBlock> {
        info!("block production called for height {:?}", height);
        Ok(Default::default())
    }
}
