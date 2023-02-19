use crate::graphql_api::ports::DatabasePort;
use fuel_core_storage::{
    iter::{
        BoxedIter,
        IntoBoxedIter,
        IterDirection,
    },
    not_found,
    tables::Coins,
    Result as StorageResult,
    StorageAsRef,
};
use fuel_core_types::{
    entities::coin::Coin,
    fuel_tx::UtxoId,
    fuel_types::Address,
};

pub trait CoinQueryData: Send + Sync {
    fn coin(&self, utxo_id: UtxoId) -> StorageResult<Coin>;

    fn owned_coins_ids(
        &self,
        owner: &Address,
        start_coin: Option<UtxoId>,
        direction: IterDirection,
    ) -> BoxedIter<StorageResult<UtxoId>>;

    fn owned_coins(
        &self,
        owner: &Address,
        start_coin: Option<UtxoId>,
        direction: IterDirection,
    ) -> BoxedIter<StorageResult<Coin>>;
}

impl<D: DatabasePort> CoinQueryData for D {
    fn coin(&self, utxo_id: UtxoId) -> StorageResult<Coin> {
        let coin = self
            .storage::<Coins>()
            .get(&utxo_id)?
            .ok_or(not_found!(Coins))?
            .into_owned();

        Ok(coin.uncompress(utxo_id))
    }

    fn owned_coins_ids(
        &self,
        owner: &Address,
        start_coin: Option<UtxoId>,
        direction: IterDirection,
    ) -> BoxedIter<StorageResult<UtxoId>> {
        self.owned_coins_ids(owner, start_coin, direction)
            .into_boxed()
    }

    fn owned_coins(
        &self,
        owner: &Address,
        start_coin: Option<UtxoId>,
        direction: IterDirection,
    ) -> BoxedIter<StorageResult<Coin>> {
        self.owned_coins_ids(owner, start_coin, direction)
            .map(|res| res.and_then(|id| self.coin(id)))
            .into_boxed()
    }
}
