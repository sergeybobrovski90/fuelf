use crate::{
    ports::{
        BlockImporter,
        PeerToPeer,
        TxPoolDb,
    },
    transaction_selector::select_transactions,
    txpool::{
        check_single_tx,
        check_transactions,
    },
    Config,
    Error as TxPoolError,
    TxInfo,
    TxPool,
};

use fuel_core_p2p::request_response::messages::{
    RequestMessage,
    MAX_REQUEST_SIZE,
};

use fuel_core_p2p::PeerId;

use fuel_core_services::{
    stream::BoxStream,
    RunnableService,
    RunnableTask,
    Service as _,
    ServiceRunner,
    StateWatcher,
};
use fuel_core_types::{
    fuel_tx::{
        ConsensusParameters,
        Transaction,
        TxId,
        UniqueIdentifier,
    },
    fuel_types::{
        BlockHeight,
        Bytes32,
    },
    services::{
        block_importer::ImportResult,
        p2p::{
            GossipData,
            GossipsubMessageAcceptance,
            GossipsubMessageInfo,
            TransactionGossipData,
        },
        txpool::{
            ArcPoolTx,
            Error,
            InsertionResult,
            TransactionStatus,
        },
    },
    tai64::Tai64,
};

use parking_lot::Mutex as ParkingMutex;
use std::sync::Arc;
use tokio::{
    sync::broadcast,
    time::MissedTickBehavior,
};
use tokio_stream::StreamExt;
use update_sender::UpdateSender;

use self::update_sender::{
    MpscChannel,
    TxStatusStream,
};

mod update_sender;

// Main transaction pool service. It is mainly responsible for
// listening for new transactions being gossiped through the network and
// adding those to its own pool.
pub type Service<P2P, DB> = ServiceRunner<Task<P2P, DB>>;
// Sidecar service that is responsible for a request-response style of syncing
// pending transactions with other peers upon connection. In other words,
// when a new connection between peers happen, they will compare their
// pending transactions and sync them with each other.
pub type TxPoolSyncService<P2P, DB> = ServiceRunner<TxPoolSyncTask<P2P, DB>>;

#[derive(Clone)]
pub struct TxStatusChange {
    new_tx_notification_sender: broadcast::Sender<TxId>,
    update_sender: UpdateSender,
}

impl TxStatusChange {
    pub fn new(capacity: usize) -> Self {
        let (new_tx_notification_sender, _) = broadcast::channel(capacity);
        let update_sender = UpdateSender::new(capacity);
        Self {
            new_tx_notification_sender,
            update_sender,
        }
    }

    pub fn send_complete(
        &self,
        id: Bytes32,
        block_height: &BlockHeight,
        message: impl Into<TxStatusMessage>,
    ) {
        tracing::info!("Transaction {id} successfully included in block {block_height}");
        self.update_sender.send(TxUpdate::new(id, message.into()));
    }

    pub fn send_submitted(&self, id: Bytes32, time: Tai64) {
        tracing::info!("Transaction {id} successfully submitted to the tx pool");
        let _ = self.new_tx_notification_sender.send(id);
        self.update_sender.send(TxUpdate::new(
            id,
            TxStatusMessage::Status(TransactionStatus::Submitted { time }),
        ));
    }

    pub fn send_squeezed_out(&self, id: Bytes32, reason: TxPoolError) {
        tracing::info!("Transaction {id} squeezed out because {reason}");
        self.update_sender.send(TxUpdate::new(
            id,
            TxStatusMessage::Status(TransactionStatus::SqueezedOut {
                reason: reason.to_string(),
            }),
        ));
    }
}

pub struct SharedState<P2P, DB> {
    tx_status_sender: TxStatusChange,
    txpool: Arc<ParkingMutex<TxPool<DB>>>,
    p2p: Arc<P2P>,
    consensus_params: ConsensusParameters,
    db: DB,
    config: Config,
}

impl<P2P, DB: Clone> Clone for SharedState<P2P, DB> {
    fn clone(&self) -> Self {
        Self {
            tx_status_sender: self.tx_status_sender.clone(),
            txpool: self.txpool.clone(),
            p2p: self.p2p.clone(),
            consensus_params: self.consensus_params.clone(),
            db: self.db.clone(),
            config: self.config.clone(),
        }
    }
}

pub struct TxPoolSyncTask<P2P, DB> {
    ttl_timer: tokio::time::Interval,
    peer_connections: BoxStream<PeerId>,
    incoming_pooled_transactions: BoxStream<Vec<Transaction>>,
    shared: SharedState<P2P, DB>,
}

#[async_trait::async_trait]
impl<P2P, DB> RunnableService for TxPoolSyncTask<P2P, DB>
where
    DB: TxPoolDb + Clone,
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData> + Send + Sync,
{
    const NAME: &'static str = "TxPoolSync";

    type SharedData = SharedState<P2P, DB>;
    type Task = TxPoolSyncTask<P2P, DB>;
    type TaskParams = ();

    fn shared_data(&self) -> Self::SharedData {
        self.shared.clone()
    }

    async fn into_task(
        mut self,
        _: &StateWatcher,
        _: Self::TaskParams,
    ) -> anyhow::Result<Self::Task> {
        self.ttl_timer.reset();
        Ok(self)
    }
}

#[async_trait::async_trait]
impl<P2P, DB> RunnableTask for TxPoolSyncTask<P2P, DB>
where
    DB: TxPoolDb,
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData> + Send + Sync,
{
    async fn run(&mut self, watcher: &mut StateWatcher) -> anyhow::Result<bool> {
        let should_continue;

        tokio::select! {
            biased;

            _ = watcher.while_started() => {
                should_continue = false;
            }

            incoming_pooled_transactions = self.incoming_pooled_transactions.next() => {
                if let Some(incoming_pooled_transactions) = incoming_pooled_transactions {
                    let current_height = self.shared.db.current_block_height()?;

                    let mut res = Vec::new();

                    for tx in incoming_pooled_transactions.into_iter() {
                        let checked_tx = match check_single_tx(tx, current_height, &self.shared.config).await {
                            Ok(tx) => tx,
                            Err(e) => {
                                tracing::error!("Unable to insert pooled transaction coming from a newly connected peer, got an {} error", e);
                                continue;
                            }
                        };
                        res.push(self.shared.txpool.lock().insert_inner(checked_tx));
                    }

                    should_continue = true;

                } else {
                    should_continue = false;
                }
            }

            new_connection = self.peer_connections.next() => {
                if let Some(peer_id) = new_connection {
                    should_continue = true;

                    let mut txs = vec![];
                    for tx in self.shared.txpool.lock().txs() {
                        txs.push(Transaction::from(&*tx.1.tx));
                    }


                    if !txs.is_empty() {
                        for txs in split_into_batches(txs) {
                            let result =  self.shared.p2p.send_pooled_transactions(peer_id, txs).await;
                            if let Err(e) = result {
                                tracing::error!("Unable to send pooled transactions, got an {} error", e);
                            }
                        }
                    }

                } else {
                    should_continue = false;
                }
            }
        }

        Ok(should_continue)
    }

    async fn shutdown(self) -> anyhow::Result<()> {
        // Nothing to shut down because we don't have any temporary state that should be dumped,
        // and we don't spawn any sub-tasks that we need to finish or await.
        // Maybe we will save and load the previous list of transactions in the future to
        // avoid losing them.

        Ok(())
    }
}

// Split transactions into batches of size less than MAX_REQUEST_SIZE.
fn split_into_batches(txs: Vec<Transaction>) -> Vec<Vec<Transaction>> {
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    let mut size = 0;
    for tx in txs.into_iter() {
        let m = RequestMessage::PooledTransactions(vec![tx.clone()]);
        let tx_size = postcard::to_stdvec(&m).unwrap().len();
        if size + tx_size < MAX_REQUEST_SIZE {
            batch.push(tx);
            size += tx_size;
        } else {
            batches.push(batch);
            batch = vec![tx];
            size = tx_size;
        }
    }
    batches.push(batch);
    batches
}

pub struct Task<P2P, DB>
where
    DB: TxPoolDb + 'static + Clone,
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData> + Send + Sync + 'static,
{
    gossiped_tx_stream: BoxStream<TransactionGossipData>,
    committed_block_stream: BoxStream<Arc<ImportResult>>,
    shared: SharedState<P2P, DB>,
    txpool_sync_task: ServiceRunner<TxPoolSyncTask<P2P, DB>>,
    ttl_timer: tokio::time::Interval,
}

#[async_trait::async_trait]
impl<P2P, DB> RunnableService for Task<P2P, DB>
where
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData> + Send + Sync,
    DB: TxPoolDb + Clone,
{
    const NAME: &'static str = "TxPool";

    type SharedData = SharedState<P2P, DB>;
    type Task = Task<P2P, DB>;
    type TaskParams = ();

    fn shared_data(&self) -> Self::SharedData {
        self.shared.clone()
    }

    async fn into_task(
        mut self,
        _: &StateWatcher,
        _: Self::TaskParams,
    ) -> anyhow::Result<Self::Task> {
        // Transaction pool sync task work as a sub service to the transaction pool task.
        // So we start it here and shut it down when this task is shut down.
        self.txpool_sync_task.start()?;
        self.ttl_timer.reset();
        Ok(self)
    }
}

#[async_trait::async_trait]
impl<P2P, DB> RunnableTask for Task<P2P, DB>
where
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData> + Send + Sync,
    DB: TxPoolDb + Clone,
{
    async fn run(&mut self, watcher: &mut StateWatcher) -> anyhow::Result<bool> {
        let should_continue;

        tokio::select! {
            biased;

            _ = watcher.while_started() => {
                should_continue = false;
            }

            _ = self.ttl_timer.tick() => {
                let removed = self.shared.txpool.lock().prune_old_txs();
                for tx in removed {
                    self.shared.tx_status_sender.send_squeezed_out(tx.id(), Error::TTLReason);
                }

                should_continue = true
            }

            result = self.committed_block_stream.next() => {
                if let Some(result) = result {
                    let block = result
                        .sealed_block
                        .entity
                        .compress(&self.shared.consensus_params.chain_id);
                    self.shared.txpool.lock().block_update(
                        &self.shared.tx_status_sender,
                        block.header().height(),
                        block.transactions()
                    );
                    should_continue = true;
                } else {
                    should_continue = false;
                }
            }

            new_transaction = self.gossiped_tx_stream.next() => {
                if let Some(GossipData { data: Some(tx), message_id, peer_id }) = new_transaction {
                    let id = tx.id(&self.shared.consensus_params.chain_id);
                    let current_height = self.shared.db.current_block_height()?;

                    // verify tx
                    let checked_tx = check_single_tx(tx, current_height, &self.shared.config).await;

                    let acceptance = match checked_tx {
                        Ok(tx) => {
                            let txs = vec![tx];

                            // insert tx
                            let mut result = tracing::info_span!("Received tx via gossip", %id)
                                .in_scope(|| {
                                    self.shared.txpool.lock().insert(
                                        &self.shared.tx_status_sender,
                                        txs
                                    )
                                });

                            match result.pop() {
                                Some(Ok(_)) => {
                                    GossipsubMessageAcceptance::Accept
                                },
                                Some(Err(_)) => {
                                    GossipsubMessageAcceptance::Reject
                                }
                                _ => GossipsubMessageAcceptance::Ignore
                            }
                        }
                        Err(_) => {
                            GossipsubMessageAcceptance::Reject
                        }
                    };

                    if acceptance != GossipsubMessageAcceptance::Ignore {
                        let message_info = GossipsubMessageInfo {
                            message_id,
                            peer_id,
                        };

                        let _ = self.shared.p2p.notify_gossip_transaction_validity(message_info, acceptance);
                    }

                    should_continue = true;
                } else {
                    should_continue = false;
                }
            }
        }
        Ok(should_continue)
    }

    async fn shutdown(self) -> anyhow::Result<()> {
        // Nothing other than the txpool sync task to shut down
        // because we don't have any temporary state that should be dumped,
        // and we don't spawn any sub-tasks that we need to finish or await.
        // Maybe we will save and load the previous list of transactions in the future to
        // avoid losing them.
        self.txpool_sync_task.stop_and_await().await?;
        Ok(())
    }
}

// TODO: Remove `find` and `find_one` methods from `txpool`. It is used only by GraphQL.
//  Instead, `fuel-core` can create a `DatabaseWithTxPool` that aggregates `TxPool` and
//  storage `Database` together. GraphQL will retrieve data from this `DatabaseWithTxPool` via
//  `StorageInspect` trait.
impl<P2P, DB> SharedState<P2P, DB>
where
    DB: TxPoolDb,
{
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

    pub fn new_tx_notification_subscribe(&self) -> broadcast::Receiver<TxId> {
        self.tx_status_sender.new_tx_notification_sender.subscribe()
    }

    pub async fn tx_update_subscribe(&self, tx_id: Bytes32) -> TxStatusStream {
        self.tx_status_sender
            .update_sender
            .subscribe::<MpscChannel>(tx_id)
            .await
    }
}

impl<P2P, DB> SharedState<P2P, DB>
where
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData>,
    DB: TxPoolDb,
{
    #[tracing::instrument(name = "insert_submitted_txn", skip_all)]
    pub async fn insert(
        &self,
        txs: Vec<Arc<Transaction>>,
    ) -> Vec<anyhow::Result<InsertionResult>> {
        // verify txs
        let block_height = self.db.current_block_height();
        let current_height = match block_height {
            Ok(val) => val,
            Err(e) => return vec![Err(e.into())],
        };

        let checked_txs = check_transactions(&txs, current_height, &self.config).await;

        let mut valid_txs = vec![];

        let checked_txs: Vec<_> = checked_txs
            .into_iter()
            .map(|tx_check| match tx_check {
                Ok(tx) => {
                    valid_txs.push(tx);
                    None
                }
                Err(err) => Some(err),
            })
            .collect();

        // insert txs
        let insertion = { self.txpool.lock().insert(&self.tx_status_sender, valid_txs) };

        for (ret, tx) in insertion.iter().zip(txs.into_iter()) {
            match ret {
                Ok(_) => {
                    let result = self.p2p.broadcast_transaction(tx.clone());
                    if let Err(e) = result {
                        // It can be only in the case of p2p being down or requests overloading it.
                        tracing::error!(
                            "Unable to broadcast transaction, got an {} error",
                            e
                        );
                    }
                }
                Err(_) => {}
            }
        }

        let mut insertion = insertion.into_iter();

        checked_txs
            .into_iter()
            .map(|check_result| match check_result {
                None => insertion.next().unwrap_or_else(|| {
                    unreachable!(
                        "the number of inserted txs matches the number of `None` results"
                    )
                }),
                Some(err) => Err(err),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct TxUpdate {
    tx_id: Bytes32,
    message: TxStatusMessage,
}

impl TxUpdate {
    pub fn new(tx_id: Bytes32, message: TxStatusMessage) -> Self {
        Self { tx_id, message }
    }

    pub fn tx_id(&self) -> &Bytes32 {
        &self.tx_id
    }

    pub fn into_msg(self) -> TxStatusMessage {
        self.message
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxStatusMessage {
    Status(TransactionStatus),
    FailedStatus,
}

pub fn new_txpool_syncing_service<P2P, DB>(
    config: Config,
    txpool: Arc<ParkingMutex<TxPool<DB>>>,
    p2p: Arc<P2P>,
    db: DB,
) -> TxPoolSyncService<P2P, DB>
where
    DB: TxPoolDb + 'static + Clone,
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData> + 'static,
{
    let mut ttl_timer = tokio::time::interval(config.transaction_ttl);
    ttl_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let number_of_active_subscription = config.number_of_active_subscription;
    let consensus_params = config.chain_config.consensus_parameters.clone();
    let tx_sync_task = TxPoolSyncTask {
        peer_connections: p2p.new_connection(),
        incoming_pooled_transactions: p2p.incoming_pooled_transactions(),
        ttl_timer,
        shared: SharedState {
            db,
            config,
            txpool,
            p2p,
            tx_status_sender: TxStatusChange::new(number_of_active_subscription),
            consensus_params,
        },
    };

    TxPoolSyncService::new(tx_sync_task)
}

pub fn new_service<P2P, Importer, DB>(
    config: Config,
    db: DB,
    importer: Importer,
    p2p: P2P,
) -> Service<P2P, DB>
where
    Importer: BlockImporter,
    P2P: PeerToPeer<GossipedTransaction = TransactionGossipData> + 'static,
    DB: TxPoolDb + Clone + 'static,
{
    let p2p = Arc::new(p2p);
    let gossiped_tx_stream = p2p.gossiped_transaction_events();
    let committed_block_stream = importer.block_events();
    let mut ttl_timer = tokio::time::interval(config.transaction_ttl);
    ttl_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let consensus_params = config.chain_config.consensus_parameters.clone();
    let number_of_active_subscription = config.number_of_active_subscription;
    let txpool = Arc::new(ParkingMutex::new(TxPool::new(config.clone(), db.clone())));

    let txpool_sync_task = new_txpool_syncing_service(
        config.clone(),
        txpool.clone(),
        p2p.clone(),
        db.clone(),
    );

    let task = Task {
        gossiped_tx_stream,
        committed_block_stream,
        txpool_sync_task,
        shared: SharedState {
            tx_status_sender: TxStatusChange::new(number_of_active_subscription),
            txpool,
            p2p,
            consensus_params,
            db,
            config,
        },
        ttl_timer,
    };

    Service::new(task)
}

impl<E> From<Result<TransactionStatus, E>> for TxStatusMessage {
    fn from(result: Result<TransactionStatus, E>) -> Self {
        match result {
            Ok(status) => TxStatusMessage::Status(status),
            _ => TxStatusMessage::FailedStatus,
        }
    }
}

#[cfg(test)]
pub mod test_helpers;
#[cfg(test)]
pub mod tests;
#[cfg(test)]
pub mod tests_p2p;
