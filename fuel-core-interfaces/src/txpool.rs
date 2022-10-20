use crate::{
    common::{
        fuel_asm::Word,
        fuel_storage::{
            StorageAsRef,
            StorageInspect,
        },
        fuel_tx::{
            field::{
                Inputs,
                Outputs,
            },
            Bytes32,
            Cacheable,
            Chargeable,
            Checked,
            ConsensusParameters,
            ContractId,
            Create,
            Fully,
            Input,
            IntoChecked,
            Output,
            Partially,
            Script,
            Transaction,
            TxId,
            UniqueIdentifier,
            UtxoId,
        },
        fuel_types::MessageId,
        fuel_vm::storage::ContractsRawCode,
    },
    db::{
        Coins,
        Error as DbStateError,
        KvStoreError,
        Messages,
    },
    model::{
        ArcTx,
        BlockHeight,
        Coin,
        Message,
        TxInfo,
    },
};
use derive_more::{
    Deref,
    DerefMut,
};
use fuel_vm::prelude::{
    Interpreter,
    PredicateStorage,
};
use std::{
    fmt::Debug,
    sync::Arc,
};
use thiserror::Error;
use tokio::sync::{
    mpsc,
    oneshot,
};

// TODO: After removing of `utxo_validation` from TxPool, use either `Fully` or `Partially`
//  and remove usage of `Either` type.
#[derive(Debug, Eq, PartialEq)]
pub enum Either<Tx: IntoChecked>
where
    Tx::Metadata: Debug + Eq,
{
    Partially(Checked<Tx, Partially>),
    Fully(Checked<Tx, Fully>),
}

/// Transaction used by the transaction pool.
#[derive(Debug, Eq, PartialEq)]
pub enum PoolTransaction {
    Script(Either<Script>),
    Create(Either<Create>),
}

impl Chargeable for PoolTransaction {
    fn price(&self) -> Word {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                script.transaction().price()
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                script.transaction().price()
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                create.transaction().price()
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                create.transaction().price()
            }
        }
    }

    fn limit(&self) -> Word {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                script.transaction().limit()
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                script.transaction().limit()
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                create.transaction().limit()
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                create.transaction().limit()
            }
        }
    }

    fn metered_bytes_size(&self) -> usize {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                script.transaction().metered_bytes_size()
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                script.transaction().metered_bytes_size()
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                create.transaction().metered_bytes_size()
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                create.transaction().metered_bytes_size()
            }
        }
    }
}

impl UniqueIdentifier for PoolTransaction {
    fn id(&self) -> Bytes32 {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                script.transaction().id()
            }
            PoolTransaction::Script(Either::Fully(script)) => script.transaction().id(),
            PoolTransaction::Create(Either::Partially(create)) => {
                create.transaction().id()
            }
            PoolTransaction::Create(Either::Fully(create)) => create.transaction().id(),
        }
    }
}

impl PoolTransaction {
    pub fn is_computed(&self) -> bool {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                script.transaction().is_computed()
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                script.transaction().is_computed()
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                create.transaction().is_computed()
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                create.transaction().is_computed()
            }
        }
    }

    pub fn inputs(&self) -> &Vec<Input> {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                script.transaction().inputs()
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                script.transaction().inputs()
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                create.transaction().inputs()
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                create.transaction().inputs()
            }
        }
    }

    pub fn outputs(&self) -> &Vec<Output> {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                script.transaction().outputs()
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                script.transaction().outputs()
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                create.transaction().outputs()
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                create.transaction().outputs()
            }
        }
    }

    pub fn max_gas(&self) -> Word {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                script.metadata().fee.max_gas()
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                script.metadata().fee.max_gas()
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                create.metadata().fee.max_gas()
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                create.metadata().fee.max_gas()
            }
        }
    }

    pub fn check_predicates(&self, params: ConsensusParameters) -> bool {
        match self {
            PoolTransaction::Script(Either::Partially(script)) => {
                Interpreter::<PredicateStorage>::check_predicates(script.clone(), params)
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                Interpreter::<PredicateStorage>::check_predicates(script.clone(), params)
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                Interpreter::<PredicateStorage>::check_predicates(create.clone(), params)
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                Interpreter::<PredicateStorage>::check_predicates(create.clone(), params)
            }
        }
    }
}

impl From<&PoolTransaction> for Transaction {
    fn from(tx: &PoolTransaction) -> Self {
        match tx {
            PoolTransaction::Script(Either::Partially(script)) => {
                Transaction::Script(script.transaction().clone())
            }
            PoolTransaction::Script(Either::Fully(script)) => {
                Transaction::Script(script.transaction().clone())
            }
            PoolTransaction::Create(Either::Partially(create)) => {
                Transaction::Create(create.transaction().clone())
            }
            PoolTransaction::Create(Either::Fully(create)) => {
                Transaction::Create(create.transaction().clone())
            }
        }
    }
}

impl From<Checked<Script, Partially>> for PoolTransaction {
    fn from(checked: Checked<Script, Partially>) -> Self {
        Self::Script(Either::Partially(checked))
    }
}

/// The `removed` field contains the list of removed transactions during the insertion
/// of the `inserted` transaction.
#[derive(Debug)]
pub struct InsertionResult {
    pub inserted: ArcTx,
    pub removed: Vec<ArcTx>,
}

pub trait TxPoolDb:
    StorageInspect<Coins, Error = KvStoreError>
    + StorageInspect<ContractsRawCode, Error = DbStateError>
    + StorageInspect<Messages, Error = KvStoreError>
    + Send
    + Sync
{
    fn utxo(&self, utxo_id: &UtxoId) -> Result<Option<Coin>, KvStoreError> {
        self.storage::<Coins>()
            .get(utxo_id)
            .map(|t| t.map(|t| t.as_ref().clone()))
    }

    fn contract_exist(&self, contract_id: &ContractId) -> Result<bool, DbStateError> {
        self.storage::<ContractsRawCode>().contains_key(contract_id)
    }

    fn message(&self, message_id: &MessageId) -> Result<Option<Message>, KvStoreError> {
        self.storage::<Messages>()
            .get(message_id)
            .map(|t| t.map(|t| t.as_ref().clone()))
    }

    fn current_block_height(&self) -> Result<BlockHeight, KvStoreError>;
}

/// RPC client for doing calls to the TxPool through an MPSC channel.
#[derive(Clone, Deref, DerefMut)]
pub struct Sender(mpsc::Sender<TxPoolMpsc>);

impl Sender {
    pub fn new(sender: mpsc::Sender<TxPoolMpsc>) -> Self {
        Self(sender)
    }

    pub async fn insert(
        &self,
        txs: Vec<Arc<Transaction>>,
    ) -> anyhow::Result<Vec<anyhow::Result<InsertionResult>>> {
        let (response, receiver) = oneshot::channel();
        self.send(TxPoolMpsc::Insert { txs, response }).await?;
        receiver.await.map_err(Into::into)
    }

    pub async fn find(&self, ids: Vec<TxId>) -> anyhow::Result<Vec<Option<TxInfo>>> {
        let (response, receiver) = oneshot::channel();
        self.send(TxPoolMpsc::Find { ids, response }).await?;
        receiver.await.map_err(Into::into)
    }

    pub async fn find_one(&self, id: TxId) -> anyhow::Result<Option<TxInfo>> {
        let (response, receiver) = oneshot::channel();
        self.send(TxPoolMpsc::FindOne { id, response }).await?;
        receiver.await.map_err(Into::into)
    }

    pub async fn find_dependent(&self, ids: Vec<TxId>) -> anyhow::Result<Vec<ArcTx>> {
        let (response, receiver) = oneshot::channel();
        self.send(TxPoolMpsc::FindDependent { ids, response })
            .await?;
        receiver.await.map_err(Into::into)
    }

    pub async fn filter_by_negative(&self, ids: Vec<TxId>) -> anyhow::Result<Vec<TxId>> {
        let (response, receiver) = oneshot::channel();
        self.send(TxPoolMpsc::FilterByNegative { ids, response })
            .await?;
        receiver.await.map_err(Into::into)
    }

    pub async fn includable(&self) -> anyhow::Result<Vec<ArcTx>> {
        let (response, receiver) = oneshot::channel();
        self.send(TxPoolMpsc::Includable { response }).await?;
        receiver.await.map_err(Into::into)
    }

    pub async fn remove(&self, ids: Vec<TxId>) -> anyhow::Result<Vec<ArcTx>> {
        let (response, receiver) = oneshot::channel();
        self.send(TxPoolMpsc::Remove { ids, response }).await?;
        receiver.await.map_err(Into::into)
    }

    pub fn channel(buffer: usize) -> (Sender, mpsc::Receiver<TxPoolMpsc>) {
        let (sender, reciever) = mpsc::channel(buffer);
        (Sender(sender), reciever)
    }
}

#[async_trait::async_trait]
impl super::poa_coordinator::TransactionPool for Sender {
    async fn total_consumable_gas(&self) -> anyhow::Result<u64> {
        let (response, receiver) = oneshot::channel();
        self.send(TxPoolMpsc::ConsumableGas { response }).await?;
        receiver.await.map_err(Into::into)
    }
}

/// RPC commands that can be sent to the TxPool through an MPSC channel.
/// Responses are returned using `response` oneshot channel.
#[derive(Debug)]
pub enum TxPoolMpsc {
    /// The amount of gas in all includable transactions combined
    ConsumableGas { response: oneshot::Sender<u64> },
    /// Return all sorted transactions that are includable in next block.
    /// This is going to be heavy operation, use it only when needed.
    Includable {
        response: oneshot::Sender<Vec<ArcTx>>,
    },
    /// import list of transaction into txpool. All needed parents need to be known
    /// and parent->child order should be enforced in Vec, we will not do that check inside
    /// txpool and will just drop child and include only parent. Additional restrain is that
    /// child gas_price needs to be lower then parent gas_price. Transaction can be received
    /// from p2p **RespondTransactions** or from userland. Because of userland we are returning
    /// error for every insert for better user experience.
    Insert {
        txs: Vec<Arc<Transaction>>,
        response: oneshot::Sender<Vec<anyhow::Result<InsertionResult>>>,
    },
    /// find all tx by their hash
    Find {
        ids: Vec<TxId>,
        response: oneshot::Sender<Vec<Option<TxInfo>>>,
    },
    /// find one tx by its hash
    FindOne {
        id: TxId,
        response: oneshot::Sender<Option<TxInfo>>,
    },
    /// find all dependent tx and return them with requested dependencies in one list sorted by Price.
    FindDependent {
        ids: Vec<TxId>,
        response: oneshot::Sender<Vec<ArcTx>>,
    },
    /// remove transaction from pool needed on user demand. Low priority
    Remove {
        ids: Vec<TxId>,
        response: oneshot::Sender<Vec<ArcTx>>,
    },
    /// Iterate over `hashes` and return all hashes that we don't have.
    /// Needed when we receive list of new hashed from peer with
    /// **BroadcastTransactionHashes**, so txpool needs to return
    /// tx that we don't have, and request them from that particular peer.
    FilterByNegative {
        ids: Vec<TxId>,
        response: oneshot::Sender<Vec<TxId>>,
    },
    /// stop txpool
    Stop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TxStatus {
    /// Submitted into txpool.
    Submitted,
    /// Executed in fuel block.
    Executed,
    /// removed from txpool.
    SqueezedOut { reason: Error },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TxStatusBroadcast {
    pub tx: ArcTx,
    pub status: TxStatus,
}

#[derive(Error, Debug, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Error {
    #[error("Transaction is not inserted. Hash is already known")]
    NotInsertedTxKnown,
    #[error("Transaction is not inserted. Pool limit is hit, try to increase gas_price")]
    NotInsertedLimitHit,
    #[error("TxPool required that transaction contains metadata")]
    NoMetadata,
    #[error("Transaction is not inserted. The gas price is too low.")]
    NotInsertedGasPriceTooLow,
    #[error(
        "Transaction is not inserted. More priced tx {0:#x} already spend this UTXO output: {1:#x}"
    )]
    NotInsertedCollision(TxId, UtxoId),
    #[error(
        "Transaction is not inserted. More priced tx has created contract with ContractId {0:#x}"
    )]
    NotInsertedCollisionContractId(ContractId),
    #[error(
        "Transaction is not inserted. A higher priced tx {0:#x} is already spending this messageId: {1:#x}"
    )]
    NotInsertedCollisionMessageId(TxId, MessageId),
    #[error(
        "Transaction is not inserted. Dependent UTXO output is not existing: {0:#x}"
    )]
    NotInsertedOutputNotExisting(UtxoId),
    #[error("Transaction is not inserted. UTXO input contract is not existing: {0:#x}")]
    NotInsertedInputContractNotExisting(ContractId),
    #[error("Transaction is not inserted. ContractId is already taken {0:#x}")]
    NotInsertedContractIdAlreadyTaken(ContractId),
    #[error("Transaction is not inserted. UTXO is not existing: {0:#x}")]
    NotInsertedInputUtxoIdNotExisting(UtxoId),
    #[error("Transaction is not inserted. UTXO is spent: {0:#x}")]
    NotInsertedInputUtxoIdSpent(UtxoId),
    #[error("Transaction is not inserted. Message is spent: {0:#x}")]
    NotInsertedInputMessageIdSpent(MessageId),
    #[error("Transaction is not inserted. Message id {0:#x} does not match any received message from the DA layer.")]
    NotInsertedInputMessageUnknown(MessageId),
    #[error(
        "Transaction is not inserted. UTXO requires Contract input {0:#x} that is priced lower"
    )]
    NotInsertedContractPricedLower(ContractId),
    #[error("Transaction is not inserted. Input output mismatch. Coin owner is different from expected input")]
    NotInsertedIoWrongOwner,
    #[error("Transaction is not inserted. Input output mismatch. Coin output does not match expected input")]
    NotInsertedIoWrongAmount,
    #[error("Transaction is not inserted. Input output mismatch. Coin output asset_id does not match expected inputs")]
    NotInsertedIoWrongAssetId,
    #[error("Transaction is not inserted. The computed message id doesn't match the provided message id.")]
    NotInsertedIoWrongMessageId,
    #[error(
        "Transaction is not inserted. Input output mismatch. Expected coin but output is contract"
    )]
    NotInsertedIoContractOutput,
    #[error(
        "Transaction is not inserted. Input output mismatch. Expected coin but output is message"
    )]
    NotInsertedIoMessageInput,
    #[error("Transaction is not inserted. Maximum depth of dependent transaction chain reached")]
    NotInsertedMaxDepth,
    #[error("Transaction exceeds the max gas per block limit. Tx gas: {tx_gas}, block limit {block_limit}")]
    NotInsertedMaxGasLimit { tx_gas: Word, block_limit: Word },
    // small todo for now it can pass but in future we should include better messages
    #[error("Transaction removed.")]
    Removed,
}
