use crate::{Config, TxPool};
use anyhow::anyhow;
use fuel_core_interfaces::txpool::{self, TxPoolDb, TxPoolMpsc, TxStatusBroadcast};
use fuel_core_interfaces::{
    block_importer::ImportBlockBroadcast,
    p2p::{TransactionBroadcast},
};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;

pub struct ServiceBuilder {
    config: Config,
    db: Option<Box<dyn TxPoolDb>>,
    txpool_sender: txpool::Sender,
    txpool_receiver: mpsc::Receiver<TxPoolMpsc>,
    tx_status_sender: broadcast::Sender<TxStatusBroadcast>,
    import_block_receiver: Option<broadcast::Receiver<ImportBlockBroadcast>>,
    local_tx_receiver: Option<broadcast::Receiver<TransactionBroadcast>>,
}

impl Default for ServiceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceBuilder {
    pub fn new() -> Self {
        let (txpool_sender, txpool_receiver) = txpool::channel(100);
        let (tx_status_sender, _receiver) = broadcast::channel(100);
        Self {
            config: Default::default(),
            db: None,
            txpool_sender,
            txpool_receiver,
            tx_status_sender,
            import_block_receiver: None,
            local_tx_receiver: None,
        }
    }

    pub fn sender(&self) -> &txpool::Sender {
        &self.txpool_sender
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TxStatusBroadcast> {
        self.tx_status_sender.subscribe()
    }

    pub fn db(&mut self, db: Box<dyn TxPoolDb>) -> &mut Self {
        self.db = Some(db);
        self
    }

    pub fn local_tx_receiver(
        &mut self,
        local_tx_receiver: broadcast::Receiver<TransactionBroadcast>,
    ) -> &mut Self {
        self.local_tx_receiver = Some(local_tx_receiver);
        self
    }

    pub fn import_block_event(
        &mut self,
        import_block_event: broadcast::Receiver<ImportBlockBroadcast>,
    ) -> &mut Self {
        self.import_block_receiver = Some(import_block_event);
        self
    }

    pub fn config(&mut self, config: Config) -> &mut Self {
        self.config = config;
        self
    }

    pub fn build(self) -> anyhow::Result<Service> {
        if self.db.is_none()
            || self.import_block_receiver.is_none()
            || self.local_tx_receiver.is_none()
        {
            return Err(anyhow!("One of context items are not set"));
        }
        let service = Service::new(
            self.txpool_sender,
            self.tx_status_sender.clone(),
            Context {
                db: Arc::new(self.db.unwrap()),
                txpool_receiver: self.txpool_receiver,
                tx_status_sender: self.tx_status_sender,
                import_block_receiver: self.import_block_receiver.unwrap(),
                config: self.config,
                local_tx_receiver: self.local_tx_receiver.unwrap(),
            },
        )?;
        Ok(service)
    }
}

pub struct Context {
    pub config: Config,
    pub db: Arc<Box<dyn TxPoolDb>>,
    pub txpool_receiver: mpsc::Receiver<TxPoolMpsc>,
    pub tx_status_sender: broadcast::Sender<TxStatusBroadcast>,
    pub import_block_receiver: broadcast::Receiver<ImportBlockBroadcast>,
    pub local_tx_receiver: broadcast::Receiver<TransactionBroadcast>,
}

impl Context {
    pub async fn run(mut self) -> Self {
        let txpool = Arc::new(RwLock::new(TxPool::new(self.config.clone())));

        loop {
            tokio::select! {
                new_transaction = self.local_tx_receiver.recv() => {
                    let db = self.db.clone();
                    let txpool = txpool.clone();

                    tokio::spawn( async move {
                        let txpool = txpool.as_ref();
                        match new_transaction.unwrap() {
                            TransactionBroadcast::NewTransaction ( tx ) => {
                                TxPool::insert_local(txpool, db.as_ref().as_ref(), Arc::new(tx)).await
                            }
                        }
                    });
                }

                event = self.txpool_receiver.recv() => {
                    if matches!(event,Some(TxPoolMpsc::Stop) | None) {
                        break;
                    }
                    let tx_status_sender = self.tx_status_sender.clone();
                    let db = self.db.clone();
                    let txpool = txpool.clone();

                    // This is litle bit risky but we can always add semaphore to limit number of requests.
                    tokio::spawn( async move {
                        let txpool = txpool.as_ref();
                    match event.unwrap() {
                        TxPoolMpsc::Includable { response } => {
                            let _ = response.send(TxPool::includable(txpool).await);
                        }
                        TxPoolMpsc::Insert { txs, response } => {
                            let _ = response.send(TxPool::insert(txpool, db.as_ref().as_ref(), tx_status_sender, txs).await);
                        }
                        TxPoolMpsc::Find { ids, response } => {
                            let _ = response.send(TxPool::find(txpool,&ids).await);
                        }
                        TxPoolMpsc::FindOne { id, response } => {
                            let _ = response.send(TxPool::find_one(txpool,&id).await);
                        }
                        TxPoolMpsc::FindDependent { ids, response } => {
                            let _ = response.send(TxPool::find_dependent(txpool,&ids).await);
                        }
                        TxPoolMpsc::FilterByNegative { ids, response } => {
                            let _ = response.send(TxPool::filter_by_negative(txpool,&ids).await);
                        }
                        TxPoolMpsc::Remove { ids } => {
                            TxPool::
                                remove(txpool, tx_status_sender, &ids).await;
                        }
                        TxPoolMpsc::Stop => {}
                    }});
                }
                _block_updated = self.import_block_receiver.recv() => {
                    let txpool = txpool.clone();
                    tokio::spawn( async move {
                        TxPool::block_update(txpool.as_ref()).await
                    });
                }
            }
        }
        self
    }
}

pub struct Service {
    txpool_sender: txpool::Sender,
    tx_status_sender: broadcast::Sender<TxStatusBroadcast>,
    join: Mutex<Option<JoinHandle<Context>>>,
    context: Arc<Mutex<Option<Context>>>,
}

impl Service {
    pub fn new(
        txpool_sender: txpool::Sender,
        tx_status_sender: broadcast::Sender<TxStatusBroadcast>,
        context: Context,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            txpool_sender,
            tx_status_sender,
            join: Mutex::new(None),
            context: Arc::new(Mutex::new(Some(context))),
        })
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        let mut join = self.join.lock().await;
        if join.is_none() {
            if let Some(context) = self.context.lock().await.take() {
                *join = Some(tokio::spawn(async { context.run().await }));
                Ok(())
            } else {
                Err(anyhow!("Starting TxPool service that is stopping"))
            }
        } else {
            Err(anyhow!("Service TxPool is already started"))
        }
    }

    pub async fn stop(&self) -> Option<JoinHandle<()>> {
        let mut join = self.join.lock().await;
        let join_handle = join.take();
        if let Some(join_handle) = join_handle {
            let _ = self.txpool_sender.send(TxPoolMpsc::Stop).await;
            let context = self.context.clone();
            Some(tokio::spawn(async move {
                let ret = join_handle.await;
                *context.lock().await = ret.ok();
            }))
        } else {
            None
        }
    }

    pub fn subscribe_ch(&self) -> broadcast::Receiver<TxStatusBroadcast> {
        self.tx_status_sender.subscribe()
    }

    pub fn sender(&self) -> &txpool::Sender {
        &self.txpool_sender
    }
}

#[cfg(any(test))]
pub mod tests {
    use super::*;
    use fuel_core_interfaces::{
        db::helpers::*,
        txpool::{Error as TxpoolError, TxStatus},
    };
    use tokio::sync::{oneshot};

    #[tokio::test]
    async fn test_start_stop() {
        let config = Config::default();
        let db = Box::new(DummyDb::filled());
        let (bs, _br) = broadcast::channel(10);

        // Meant to simulate p2p's channels which hook in to communicate with txpool
        let (_, local_tx_receiver) = broadcast::channel(100);

        let mut builder = ServiceBuilder::new();
        builder
            .config(config)
            .db(db)
            .local_tx_receiver(local_tx_receiver)
            .import_block_event(bs.subscribe());
        let service = builder.build().unwrap();

        assert!(service.start().await.is_ok(), "start service");

        // Double start will return false.
        assert!(service.start().await.is_err(), "double start should fail");

        let stop_handle = service.stop().await;
        assert!(stop_handle.is_some());
        let _ = stop_handle.unwrap().await;

        assert!(service.start().await.is_ok(), "Should start again");
    }

    #[tokio::test]
    async fn test_filter_by_negative() {
        let config = Config::default();
        let db = Box::new(DummyDb::filled());
        let (_bs, br) = broadcast::channel(10);

        // Meant to simulate p2p's channels which hook in to communicate with txpool
        let (_, local_tx_receiver) = broadcast::channel(100);

        let tx1_hash = *TX_ID1;
        let tx2_hash = *TX_ID2;
        let tx3_hash = *TX_ID3;

        let tx1 = Arc::new(DummyDb::dummy_tx(tx1_hash));
        let tx2 = Arc::new(DummyDb::dummy_tx(tx2_hash));

        let mut builder = ServiceBuilder::new();
        builder
            .config(config)
            .db(db)
            .local_tx_receiver(local_tx_receiver)
            .import_block_event(br);
        let service = builder.build().unwrap();
        service.start().await.ok();

        let (response, receiver) = oneshot::channel();
        let _ = service
            .sender()
            .send(TxPoolMpsc::Insert {
                txs: vec![tx1, tx2],
                response,
            })
            .await;
        let out = receiver.await.unwrap();

        assert_eq!(out.len(), 2, "Shoud be len 2:{:?}", out);
        assert!(out[0].is_ok(), "Tx1 should be OK, got err:{:?}", out);
        assert!(out[1].is_ok(), "Tx2 should be OK, got err:{:?}", out);

        let (response, receiver) = oneshot::channel();
        let _ = service
            .sender()
            .send(TxPoolMpsc::FilterByNegative {
                ids: vec![tx1_hash, tx3_hash],
                response,
            })
            .await;
        let out = receiver.await.unwrap();

        assert_eq!(out.len(), 1, "Shoud be len 1:{:?}", out);
        assert_eq!(out[0], tx3_hash, "Found tx id match{:?}", out);
        service.stop().await.unwrap().await.unwrap();
    }

    #[tokio::test]
    async fn test_find() {
        let config = Config::default();
        let db = Box::new(DummyDb::filled());
        let (_bs, br) = broadcast::channel(10);

        // Meant to simulate p2p's channels which hook in to communicate with txpool
        let (_, local_tx_receiver) = broadcast::channel(100);

        let tx1_hash = *TX_ID1;
        let tx2_hash = *TX_ID2;
        let tx3_hash = *TX_ID3;

        let tx1 = Arc::new(DummyDb::dummy_tx(tx1_hash));
        let tx2 = Arc::new(DummyDb::dummy_tx(tx2_hash));

        let mut builder = ServiceBuilder::new();
        builder
            .config(config)
            .db(db)
            .local_tx_receiver(local_tx_receiver)
            .import_block_event(br);
        let service = builder.build().unwrap();
        service.start().await.ok();

        let (response, receiver) = oneshot::channel();
        let _ = service
            .sender()
            .send(TxPoolMpsc::Insert {
                txs: vec![tx1, tx2],
                response,
            })
            .await;
        let out = receiver.await.unwrap();

        assert_eq!(out.len(), 2, "Shoud be len 2:{:?}", out);
        assert!(out[0].is_ok(), "Tx1 should be OK, got err:{:?}", out);
        assert!(out[1].is_ok(), "Tx2 should be OK, got err:{:?}", out);
        let (response, receiver) = oneshot::channel();
        let _ = service
            .sender()
            .send(TxPoolMpsc::Find {
                ids: vec![tx1_hash, tx3_hash],
                response,
            })
            .await;
        let out = receiver.await.unwrap();
        assert_eq!(out.len(), 2, "Shoud be len 2:{:?}", out);
        assert!(out[0].is_some(), "Tx1 should be some:{:?}", out);
        let id = out[0].as_ref().unwrap().id();
        assert_eq!(id, tx1_hash, "Found tx id match{:?}", out);
        assert!(out[1].is_none(), "Tx3 should not be found:{:?}", out);
        service.stop().await.unwrap().await.unwrap();
    }

    #[tokio::test]
    async fn test_insert_from_p2p() {
        let config = Config::default();
        let db = Box::new(DummyDb::filled());
        let (_bs, br) = broadcast::channel(10);

        // Meant to simulate p2p's channels which hook in to communicate with txpool
        let (local_tx_sender, local_tx_receiver) = broadcast::channel(100);

        let tx1_hash = *TX_ID1;
        let tx1 = DummyDb::dummy_tx(tx1_hash);

        let mut builder = ServiceBuilder::new();
        builder
            .config(config)
            .db(db)
            .local_tx_receiver(local_tx_receiver)
            .import_block_event(br);
        let service = builder.build().unwrap();
        service.start().await.ok();

        let broadcast_tx = TransactionBroadcast::NewTransaction(tx1.clone());
        let res = local_tx_sender.send(broadcast_tx).unwrap();

        assert_eq!(1, res);

        let (response, receiver) = oneshot::channel();

        let _ = service
            .sender()
            .send(TxPoolMpsc::Find {
                ids: vec![tx1_hash],
                response,
            })
            .await;
        let out = receiver.await.unwrap();

        let arc_tx1 = Arc::new(tx1);

        assert_eq!(arc_tx1, *out[0].as_ref().unwrap().tx());
    }

    #[tokio::test]
    async fn test_simple_insert_removal_subscription() {
        let config = Config::default();
        let db = Box::new(DummyDb::filled());
        let (_bs, br) = broadcast::channel(10);

        // Meant to simulate p2p's channels which hook in to communicate with txpool
        let (_, local_tx_receiver) = broadcast::channel(100);

        let tx1_hash = *TX_ID1;
        let tx2_hash = *TX_ID2;

        let tx1 = Arc::new(DummyDb::dummy_tx(tx1_hash));
        let tx2 = Arc::new(DummyDb::dummy_tx(tx2_hash));

        let mut builder = ServiceBuilder::new();
        builder
            .config(config)
            .db(db)
            .local_tx_receiver(local_tx_receiver)
            .import_block_event(br);
        let service = builder.build().unwrap();
        service.start().await.ok();

        let mut subscribe = service.subscribe_ch();

        let (response, receiver) = oneshot::channel();
        let _ = service
            .sender()
            .send(TxPoolMpsc::Insert {
                txs: vec![tx1.clone(), tx2.clone()],
                response,
            })
            .await;
        let out = receiver.await.unwrap();

        assert!(out[0].is_ok(), "Tx1 should be OK, got err:{:?}", out);
        assert!(out[1].is_ok(), "Tx2 should be OK, got err:{:?}", out);

        // we are sure that included tx are already broadcasted.
        assert_eq!(
            subscribe.try_recv(),
            Ok(TxStatusBroadcast {
                tx: tx1.clone(),
                status: TxStatus::Submitted,
            }),
            "First added should be tx1"
        );
        assert_eq!(
            subscribe.try_recv(),
            Ok(TxStatusBroadcast {
                tx: tx2.clone(),
                status: TxStatus::Submitted,
            }),
            "Second added should be tx2"
        );

        // remove them
        let _ = service
            .sender()
            .send(TxPoolMpsc::Remove {
                ids: vec![tx1_hash, tx2_hash],
            })
            .await;

        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), subscribe.recv()).await,
            Ok(Ok(TxStatusBroadcast {
                tx: tx1,
                status: TxStatus::SqueezedOut {
                    reason: TxpoolError::Removed
                }
            })),
            "First removed should be tx1"
        );

        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), subscribe.recv()).await,
            Ok(Ok(TxStatusBroadcast {
                tx: tx2,
                status: TxStatus::SqueezedOut {
                    reason: TxpoolError::Removed
                }
            })),
            "Second removed should be tx2"
        );
    }
}
