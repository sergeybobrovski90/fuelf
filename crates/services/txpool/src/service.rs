use crate::{
    ports::{
        BlockImport,
        PeerToPeer,
        TxPoolDb,
    },
    transaction_selector::select_transactions,
    Config,
    Error as TxPoolError,
    TxPool,
};
use anyhow::anyhow;
use fuel_core_services::{
    RunnableService,
    ServiceRunner,
};
use fuel_core_types::{
    fuel_tx::{
        Transaction,
        TxId,
    },
    fuel_types::Bytes32,
    services::{
        p2p::{
            GossipData,
            TransactionGossipData,
        },
        txpool::{
            ArcPoolTx,
            InsertionResult,
            TxInfo,
            TxStatus,
        },
    },
};
use parking_lot::Mutex as ParkingMutex;
use std::sync::Arc;
use tokio::sync::broadcast;

pub type Service = ServiceRunner<Context>;

type PeerToPeerForTx = Box<dyn PeerToPeer<GossipedTransaction = TransactionGossipData>>;

pub struct ServiceBuilder {
    config: Config,
    db: Option<Arc<dyn TxPoolDb>>,
    tx_status_sender: Option<TxStatusChange>,
    importer: Option<Box<dyn BlockImport>>,
    p2p_port: Option<PeerToPeerForTx>,
}

#[derive(Clone)]
pub struct TxStatusChange {
    status_sender: broadcast::Sender<TxStatus>,
    update_sender: broadcast::Sender<TxUpdate>,
}

impl TxStatusChange {
    pub fn new(capacity: usize) -> Self {
        let (status_sender, _) = broadcast::channel(capacity);
        let (update_sender, _) = broadcast::channel(capacity);
        Self {
            status_sender,
            update_sender,
        }
    }
    pub fn send_complete(&self, id: Bytes32) {
        let _ = self.status_sender.send(TxStatus::Completed);
        self.updated(id);
    }

    pub fn send_submitted(&self, id: Bytes32) {
        let _ = self.status_sender.send(TxStatus::Submitted);
        self.updated(id);
    }

    pub fn send_squeezed_out(&self, id: Bytes32, reason: TxPoolError) {
        let _ = self.status_sender.send(TxStatus::SqueezedOut {
            reason: reason.clone(),
        });
        let _ = self.update_sender.send(TxUpdate::squeezed_out(id, reason));
    }

    fn updated(&self, id: Bytes32) {
        let _ = self.update_sender.send(TxUpdate::updated(id));
    }
}

impl Default for ServiceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceBuilder {
    pub fn new() -> Self {
        Self {
            config: Default::default(),
            db: None,
            tx_status_sender: None,
            importer: None,
            p2p_port: None,
        }
    }

    pub fn tx_status_subscribe(&self) -> broadcast::Receiver<TxStatus> {
        self.tx_status_sender
            .as_ref()
            .unwrap()
            .status_sender
            .subscribe()
    }

    pub fn tx_change_subscribe(&self) -> broadcast::Receiver<TxUpdate> {
        self.tx_status_sender
            .as_ref()
            .unwrap()
            .update_sender
            .subscribe()
    }

    pub fn db(&mut self, db: Arc<dyn TxPoolDb>) -> &mut Self {
        self.db = Some(db);
        self
    }

    pub fn tx_status_sender(&mut self, tx_status_sender: TxStatusChange) -> &mut Self {
        self.tx_status_sender = Some(tx_status_sender);
        self
    }

    pub fn p2p_port(&mut self, p2p_port: PeerToPeerForTx) -> &mut Self {
        self.p2p_port = Some(p2p_port);
        self
    }

    pub fn importer(&mut self, importer: Box<dyn BlockImport>) -> &mut Self {
        self.importer = Some(importer);
        self
    }

    pub fn config(&mut self, config: Config) -> &mut Self {
        self.config = config;
        self
    }

    pub fn build(self) -> anyhow::Result<Service> {
        if self.db.is_none()
            || self.importer.is_none()
            || self.p2p_port.is_none()
            || self.tx_status_sender.is_none()
        {
            return Err(anyhow!("One of context items are not set"))
        }

        let tx_status_sender = self.tx_status_sender.clone().unwrap();
        let txpool = Arc::new(ParkingMutex::new(TxPool::new(self.config)));
        let context = Context {
            importer: self.importer.unwrap(),
            p2p_port: self.p2p_port.unwrap(),
            shared: SharedState {
                tx_status_sender,
                txpool,
                db: self.db.unwrap(),
            },
        };

        Ok(Service::new(context))
    }
}

#[derive(Clone)]
pub struct SharedState {
    db: Arc<dyn TxPoolDb>,
    tx_status_sender: TxStatusChange,
    txpool: Arc<ParkingMutex<TxPool>>,
}

pub struct Context {
    importer: Box<dyn BlockImport>,
    p2p_port: PeerToPeerForTx,
    shared: SharedState,
}

#[async_trait::async_trait]
impl RunnableService for Context {
    const NAME: &'static str = "TxPool";

    type SharedData = SharedState;

    fn shared_data(&self) -> Self::SharedData {
        self.shared.clone()
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn run(&mut self) -> anyhow::Result<bool> {
        tokio::select! {
            new_transaction = self.p2p_port.next_gossiped_transaction() => {
                if let GossipData { data: Some(tx), .. } = new_transaction {
                    let txs = vec!(Arc::new(tx));
                    self.shared.txpool.lock().insert(
                        self.shared.db.as_ref(),
                        &self.shared.tx_status_sender,
                        &txs
                    );
                }
            }

            block = self.importer.next_block() => {
                self.shared.txpool.lock().block_update(&self.shared.tx_status_sender, block);
            }
        }
        Ok(true /* should_continue */)
    }
}

// TODO: Remove `find` and `find_one` methods from `txpool`. It is used only by GraphQL.
//  Instead, `fuel-core` can create a `DatabaseWithTxPool` that aggregates `TxPool` and
//  storage `Database` together. GraphQL will retrieve data from this `DatabaseWithTxPool` via
//  `StorageInspect` trait.
impl SharedState {
    pub fn pending_number(&self) -> usize {
        self.txpool.lock().pending_number()
    }

    pub fn total_consumable_gas(&self) -> u64 {
        self.txpool.lock().consumable_gas()
    }

    pub fn remove_txs(&self, ids: Vec<TxId>) -> Vec<ArcPoolTx> {
        self.txpool.lock().remove(&self.tx_status_sender, &ids)
    }

    pub fn insert(
        &self,
        txs: Vec<Arc<Transaction>>,
    ) -> Vec<anyhow::Result<InsertionResult>> {
        self.txpool
            .lock()
            .insert(self.db.as_ref(), &self.tx_status_sender, &txs)
    }

    pub fn find(&self, ids: Vec<TxId>) -> Vec<Option<TxInfo>> {
        self.txpool.lock().find(&ids)
    }

    pub fn find_one(&self, id: TxId) -> Option<TxInfo> {
        self.txpool.lock().find_one(&id)
    }

    pub fn find_dependent(&self, ids: Vec<TxId>) -> Vec<ArcPoolTx> {
        self.txpool.lock().find_dependent(&ids)
    }

    pub fn select_transactions(&self, max_gas: u64) -> Vec<ArcPoolTx> {
        let mut guard = self.txpool.lock();
        let txs = guard.includable();
        let sorted_txs = select_transactions(txs, max_gas);

        for tx in sorted_txs.iter() {
            guard.remove_committed_tx(&tx.id());
        }
        sorted_txs
    }

    pub fn remove(&self, ids: Vec<TxId>) -> Vec<ArcPoolTx> {
        self.txpool.lock().remove(&self.tx_status_sender, &ids)
    }

    pub fn tx_status_subscribe(&self) -> broadcast::Receiver<TxStatus> {
        self.tx_status_sender.status_sender.subscribe()
    }

    pub fn tx_update_subscribe(&self) -> broadcast::Receiver<TxUpdate> {
        self.tx_status_sender.update_sender.subscribe()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TxUpdate {
    tx_id: Bytes32,
    squeezed_out: Option<TxPoolError>,
}

impl TxUpdate {
    pub fn updated(tx_id: Bytes32) -> Self {
        Self {
            tx_id,
            squeezed_out: None,
        }
    }

    pub fn squeezed_out(tx_id: Bytes32, reason: TxPoolError) -> Self {
        Self {
            tx_id,
            squeezed_out: Some(reason),
        }
    }

    pub fn tx_id(&self) -> &Bytes32 {
        &self.tx_id
    }

    pub fn was_squeezed_out(&self) -> bool {
        self.squeezed_out.is_some()
    }

    pub fn into_squeezed_out_reason(self) -> Option<TxPoolError> {
        self.squeezed_out
    }
}

#[cfg(test)]
pub mod test_helpers;
#[cfg(test)]
pub mod tests;
#[cfg(test)]
pub mod tests_p2p;
