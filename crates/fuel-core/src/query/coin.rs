use crate::fuel_core_graphql_api::database::ReadView;
use fuel_core_storage::{
    iter::{
        IntoBoxedIter,
        IterDirection,
    },
    Error as StorageError,
    Result as StorageResult,
};
use fuel_core_types::{
    entities::coins::coin::Coin,
    fuel_tx::UtxoId,
    fuel_types::Address,
};
use futures::{
    Stream,
    StreamExt,
    TryStreamExt,
};

impl ReadView {
    pub fn coin(&self, utxo_id: UtxoId) -> StorageResult<Coin> {
        self.on_chain.coin(utxo_id)
    }

    pub async fn coins(
        &self,
        utxo_ids: Vec<UtxoId>,
    ) -> impl Iterator<Item = StorageResult<Coin>> + '_ {
        let coins: Vec<_> = self.on_chain.coins(utxo_ids.iter().into_boxed()).collect();

        // Give a chance for other tasks to run.
        tokio::task::yield_now().await;
        coins.into_iter()
    }

    pub fn owned_coins(
        &self,
        owner: &Address,
        start_coin: Option<UtxoId>,
        direction: IterDirection,
    ) -> impl Stream<Item = StorageResult<Coin>> + '_ {
        self.owned_coins_ids(owner, start_coin, direction)
            .chunks(self.batch_size)
            .map(|chunk| {
                use itertools::Itertools;

                let chunk = chunk.into_iter().try_collect::<_, Vec<_>, _>()?;
                Ok::<_, StorageError>(chunk)
            })
            .try_filter_map(move |chunk| async move {
                let chunk = self.coins(chunk).await;
                Ok(Some(futures::stream::iter(chunk)))
            })
            .try_flatten()
    }
}
