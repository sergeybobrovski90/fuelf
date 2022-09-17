use anyhow::Result;
use async_trait::async_trait;
use fuel_core_interfaces::{
    common::fuel_tx::CheckedTransaction,
    model::{
        BlockHeight,
        DaBlockHeight,
    },
};
use std::sync::Arc;

#[async_trait]
pub trait Relayer: Sync + Send {
    /// Get the best finalized height from the DA layer
    async fn get_best_finalized_da_height(&self) -> Result<DaBlockHeight>;
}

#[async_trait]
pub trait TxPool: Sync + Send {
    async fn get_includable_txs(
        &self,
        // could be used by the txpool to filter txs based on maturity
        block_height: BlockHeight,
    ) -> Result<Vec<Arc<CheckedTransaction>>>;
}
