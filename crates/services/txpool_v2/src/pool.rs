use fuel_core_types::{
    fuel_tx::field::BlobId,
    services::txpool::PoolTransaction,
};
use tracing::instrument;

#[cfg(test)]
use fuel_core_types::fuel_tx::TxId;
#[cfg(test)]
use std::collections::HashMap;

use crate::{
    collision_manager::{
        CollisionManager,
        CollisionManagerStorage,
    },
    config::Config,
    error::Error,
    ports::{
        AtomicView,
        TxPoolPersistentStorage,
    },
    selection_algorithms::{
        Constraints,
        SelectionAlgorithm,
        SelectionAlgorithmStorage,
    },
    storage::Storage,
};

/// The pool is the main component of the txpool service. It is responsible for storing transactions
/// and allowing the selection of transactions for inclusion in a block.
pub struct Pool<PSProvider, S: Storage, CM, SA> {
    /// Configuration of the pool.
    pub config: Config,
    /// The storage of the pool.
    storage: S,
    /// The collision manager of the pool.
    collision_manager: CM,
    /// The selection algorithm of the pool.
    selection_algorithm: SA,
    /// The persistent storage of the pool.
    persistent_storage_provider: PSProvider,
    #[cfg(test)]
    tx_id_to_storage_id: HashMap<TxId, S::StorageIndex>,
}

impl<PSProvider, S: Storage, CM, SA> Pool<PSProvider, S, CM, SA> {
    /// Create a new pool.
    pub fn new(
        persistent_storage_provider: PSProvider,
        storage: S,
        collision_manager: CM,
        selection_algorithm: SA,
        config: Config,
    ) -> Self {
        Pool {
            storage,
            collision_manager,
            selection_algorithm,
            persistent_storage_provider,
            config,
            #[cfg(test)]
            tx_id_to_storage_id: HashMap::new(),
        }
    }
}

impl<PS, View, S, CM, SA> Pool<PS, S, CM, SA>
where
    PS: AtomicView<LatestView = View>,
    View: TxPoolPersistentStorage,
    S: Storage,
    S: CollisionManagerStorage<StorageIndex = <S as Storage>::StorageIndex>,
    S: SelectionAlgorithmStorage<StorageIndex = <S as Storage>::StorageIndex>,
    CM: CollisionManager<S>,
    SA: SelectionAlgorithm<S>,
{
    /// Insert transactions into the pool.
    /// Returns a list of results for each transaction.
    /// Each result is a list of transactions that were removed from the pool
    /// because of the insertion of the new transaction.
    #[instrument(skip(self))]
    pub fn insert(
        &mut self,
        transactions: Vec<PoolTransaction>,
    ) -> Result<Vec<Result<Vec<PoolTransaction>, Error>>, Error> {
        Ok(transactions
            .into_iter()
            .map(|tx| {
                let latest_view = self
                    .persistent_storage_provider
                    .latest_view()
                    .map_err(|e| Error::Database(format!("{:?}", e)))?;
                #[cfg(test)]
                let tx_id = tx.id();
                if self.storage.count() >= self.config.max_txs {
                    return Err(Error::NotInsertedLimitHit);
                }
                self.config.black_list.check_blacklisting(&tx)?;
                if let PoolTransaction::Blob(checked_tx, _) = &tx {
                    let blob_id = checked_tx.transaction().blob_id();
                    if latest_view
                        .blob_exist(blob_id)
                        .map_err(|e| Error::Database(format!("{:?}", e)))?
                    {
                        return Err(Error::NotInsertedBlobIdAlreadyTaken(*blob_id))
                    }
                }
                let collisions = self
                    .collision_manager
                    .collect_colliding_transactions(&tx, &self.storage)?;
                let dependencies = self.storage.collect_dependencies_transactions(
                    &tx,
                    collisions.reasons,
                    &latest_view,
                    self.config.utxo_validation,
                )?;
                let has_dependencies = !dependencies.is_empty();
                let (storage_id, removed_transactions) = self.storage.store_transaction(
                    tx,
                    dependencies,
                    collisions.colliding_txs,
                )?;
                #[cfg(test)]
                {
                    self.tx_id_to_storage_id.insert(tx_id, storage_id);
                }
                // No dependencies directly in the graph and the sorted transactions
                if !has_dependencies {
                    self.selection_algorithm
                        .new_executable_transactions(vec![storage_id], &self.storage)?;
                }
                let tx = Storage::get(&self.storage, &storage_id)?;
                let result = removed_transactions
                    .into_iter()
                    .map(|tx| {
                        self.collision_manager.on_removed_transaction(&tx)?;
                        self.selection_algorithm.on_removed_transaction(&tx)?;
                        #[cfg(test)]
                        {
                            self.tx_id_to_storage_id.remove(&tx.id());
                        }
                        Ok(tx)
                    })
                    .collect();
                self.collision_manager
                    .on_stored_transaction(&tx.transaction, storage_id)?;
                result
            })
            .collect())
    }

    // TODO: Use block space also (https://github.com/FuelLabs/fuel-core/issues/2133)
    /// Extract transactions for a block.
    /// Returns a list of transactions that were selected for the block
    /// based on the constraints given in the configuration and the selection algorithm used.
    pub fn extract_transactions_for_block(
        &mut self,
    ) -> Result<Vec<PoolTransaction>, Error> {
        self.selection_algorithm
            .gather_best_txs(
                Constraints {
                    max_gas: self.config.max_block_gas,
                },
                &self.storage,
            )?
            .into_iter()
            .map(|storage_id| {
                let storage_data = self.storage.remove_transaction(storage_id)?;
                self.collision_manager
                    .on_removed_transaction(&storage_data.transaction)?;
                self.selection_algorithm
                    .on_removed_transaction(&storage_data.transaction)?;
                #[cfg(test)]
                {
                    self.tx_id_to_storage_id
                        .remove(&storage_data.transaction.id());
                }
                Ok(storage_data.transaction)
            })
            .collect()
    }

    /// Prune transactions from the pool.
    pub fn prune(&mut self) -> Result<Vec<PoolTransaction>, Error> {
        Ok(vec![])
    }

    #[cfg(test)]
    pub fn find_one(&self, tx_id: &TxId) -> Option<&PoolTransaction> {
        Storage::get(&self.storage, self.tx_id_to_storage_id.get(tx_id)?)
            .map(|data| &data.transaction)
            .ok()
    }
}
