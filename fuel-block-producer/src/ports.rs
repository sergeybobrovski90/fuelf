use async_trait::async_trait;
use fuel_core_interfaces::{
    common::fuel_tx::Receipt,
    db::DatabaseTransaction,
    executor::{
        Error as ExecutorError,
        ExecutionBlock,
        ExecutionResult,
        UncommittedResult,
    },
    model::{
        ArcPoolTx,
        BlockHeight,
        DaBlockHeight,
        FuelBlockDb,
    },
};
use std::borrow::Cow;

pub trait BlockProducerDatabase: Send + Sync {
    /// fetch previously committed block at given height
    fn get_block(
        &self,
        fuel_height: BlockHeight,
    ) -> anyhow::Result<Option<Cow<FuelBlockDb>>>;

    /// Fetch the current block height
    fn current_block_height(&self) -> anyhow::Result<BlockHeight>;
}

#[async_trait]
pub trait TxPool: Sync + Send {
    async fn get_includable_txs(
        &self,
        // could be used by the txpool to filter txs based on maturity
        block_height: BlockHeight,
        // The upper limit for the total amount of gas of these txs
        max_gas: u64,
    ) -> anyhow::Result<Vec<ArcPoolTx>>;
}

#[async_trait::async_trait]
pub trait Relayer: Sync + Send {
    /// Get the best finalized height from the DA layer
    async fn get_best_finalized_da_height(&self) -> anyhow::Result<DaBlockHeight>;
}

// TODO: Replace by the analog from the `fuel-core-storage`.
pub type DBTransaction<Database> = Box<dyn DatabaseTransaction<Database>>;

pub trait Executor<Database: ?Sized>: Sync + Send {
    /// Executes the block and commits the result of the execution into the inner `Database`.
    fn execute_and_commit(
        &self,
        block: ExecutionBlock,
    ) -> Result<ExecutionResult, ExecutorError> {
        let (result, db_transaction) = self.execute_without_commit(block)?.into();
        db_transaction.commit_box()?;
        Ok(result)
    }

    /// Executes the block and returns the result of execution with uncommitted database
    /// transaction.
    fn execute_without_commit(
        &self,
        block: ExecutionBlock,
    ) -> Result<UncommittedResult<DBTransaction<Database>>, ExecutorError>;

    fn dry_run(
        &self,
        block: ExecutionBlock,
        utxo_validation: Option<bool>,
    ) -> Result<Vec<Vec<Receipt>>, ExecutorError>;
}
