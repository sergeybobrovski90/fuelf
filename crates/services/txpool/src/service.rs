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
use fuel_core_services::{
    stream::BoxStream,
    RunnableService,
    ServiceRunner,
};
use fuel_core_types::{
    blockchain::SealedBlock,
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
use tokio_stream::StreamExt;

pub type Service<P2P> = ServiceRunner<Task<P2P>>;

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

pub struct SharedState<P2P> {
    db: Arc<dyn TxPoolDb>,
    tx_status_sender: TxStatusChange,
    txpool: Arc<ParkingMutex<TxPool>>,
    p2p: Arc<P2P>,
}

impl<P2P> Clone for SharedState<P2P> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            tx_status_sender: self.tx_status_sender.clone(),
            txpool: self.txpool.clone(),
            p2p: self.p2p.clone(),
        }
    }
}

pub struct Task<P2P> {
    gossiped_tx_stream: BoxStream<TransactionGossipData>,
    committed_block_stream: BoxStream<SealedBlock>,
    shared: SharedState<P2P>,
}

#[async_trait::async_trait]
impl<P2P> RunnableService for Task<P2P>
where
    P2P: Send + Sync,
{
    const NAME: &'static str = "TxPool";

    type SharedData = SharedState<P2P>;

    fn shared_data(&self) -> Self::SharedData {
        self.shared.clone()
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn run(&mut self) -> anyhow::Result<bool> {
        tokio::select! {
            new_transaction = self.gossiped_tx_stream.next() => {
                if let Some(GossipData { data: Some(tx), .. }) = new_transaction {
                    let txs = vec!(Arc::new(tx));
                    self.shared.txpool.lock().insert(
                        self.shared.db.as_ref(),
                        &self.shared.tx_status_sender,
                        &txs
                    );
                } else {
                    let should_continue = false;
                    return Ok(should_continue);
                }
            }

            block = self.committed_block_stream.next() => {
                if let Some(block) = block {
                    self.shared.txpool.lock().block_update(&self.shared.tx_status_sender, block);
                } else {
                    let should_continue = false;
                    return Ok(should_continue);
                }
            }
        }
        Ok(true /* should_continue */)
    }
}

// TODO: Remove `find` and `find_one` methods from `txpool`. It is used only by GraphQL.
//  Instead, `fuel-core` can create a `DatabaseWithTxPool` that aggregates `TxPool` and
//  storage `Database` together. GraphQL will retrieve data from this `DatabaseWithTxPool` via
//  `StorageInspect` trait.
impl<P2P> SharedState<P2P> {
    pub fn pending_number(&self) -> usize {
        self.txpool.lock().pending_number()
    }

    pub fn total_consumable_gas(&self) -> u64 {
        self.txpool.lock().consumable_gas()
    }

    pub fn remove_txs(&self, ids: Vec<TxId>) -> Vec<ArcPoolTx> {
        self.txpool.lock().remove(&self.tx_status_sender, &ids)
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

impl<P2P> SharedState<P2P>
where
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData>,
{
    pub fn insert(
        &self,
        txs: Vec<Arc<Transaction>>,
    ) -> Vec<anyhow::Result<InsertionResult>> {
        let insert = {
            self.txpool
                .lock()
                .insert(self.db.as_ref(), &self.tx_status_sender, &txs)
        };

        for (ret, tx) in insert.iter().zip(txs.into_iter()) {
            match ret {
                Ok(_) => {
                    let _ = self.p2p.broadcast_transaction(tx.clone());
                }
                Err(_) => {}
            }
        }
        insert
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

pub fn new_service<P2P, Importer, DB>(
    config: Config,
    db: DB,
    tx_status_sender: TxStatusChange,
    importer: Importer,
    p2p: P2P,
) -> Service<P2P>
where
    Importer: BlockImport,
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData> + 'static,
    DB: TxPoolDb + 'static,
{
    let p2p = Arc::new(p2p);
    let gossiped_tx_stream = p2p.gossiped_transaction_events();
    let committed_block_stream = importer.block_events();
    let txpool = Arc::new(ParkingMutex::new(TxPool::new(config)));
    let db: Arc<dyn TxPoolDb> = Arc::new(db);
    let task = Task {
        gossiped_tx_stream,
        committed_block_stream,
        shared: SharedState {
            db,
            tx_status_sender,
            txpool,
            p2p,
        },
    };

    Service::new(task)
}

#[cfg(test)]
pub mod test_helpers;
#[cfg(test)]
pub mod tests;
#[cfg(test)]
pub mod tests_p2p;
