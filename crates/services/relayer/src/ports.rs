//! Ports used by the relayer to access the outside world

use async_trait::async_trait;
use fuel_core_storage::Result as StorageResult;
use fuel_core_types::{
    blockchain::primitives::DaBlockHeight,
    services::relayer::Event,
};

#[cfg(test)]
mod tests;

/// Manages state related to supported external chains.
#[async_trait]
pub trait RelayerDb: Send + Sync {
    /// Add bridge events to database. Events are not revertible.
    /// Must only set a new da height if it is greater than the current.
    fn insert_events(
        &mut self,
        da_height: &DaBlockHeight,
        events: &[Event],
    ) -> StorageResult<()>;

    /// Set finalized da height that represent last block from da layer that got finalized.
    /// This will only set the value if it is greater than the current.
    fn set_finalized_da_height_to_at_least(
        &mut self,
        block: &DaBlockHeight,
    ) -> StorageResult<()>;

    /// Get finalized da height that represent last block from da layer that got finalized.
    /// Panics if height is not set as of initialization of database.
    fn get_finalized_da_height(&self) -> StorageResult<DaBlockHeight>;
}
