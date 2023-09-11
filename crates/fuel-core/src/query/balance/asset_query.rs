use crate::{
    graphql_api::service::Database,
    query::{
        CoinQueryData,
        MessageQueryData,
    },
};
use fuel_core_storage::{
    iter::IterDirection,
    Error as StorageError,
    Result as StorageResult,
};
use fuel_core_types::{
    entities::coins::{
        CoinId,
        CoinType,
    },
    fuel_types::{
        Address,
        AssetId,
    },
};
use itertools::Itertools;
use std::collections::HashSet;

/// At least required `target` of the query per asset's `id` with `max` coins.
#[derive(Clone)]
pub struct AssetSpendTarget {
    pub id: AssetId,
    pub target: u64,
    pub max: usize,
}

impl AssetSpendTarget {
    pub fn new(id: AssetId, target: u64, max: u64) -> Self {
        Self {
            id,
            target,
            max: max as usize,
        }
    }
}

#[derive(Default)]
pub struct Exclude {
    pub coin_ids: HashSet<CoinId>,
}

impl Exclude {
    pub fn new(ids: Vec<CoinId>) -> Self {
        let mut instance = Self::default();

        for id in ids.into_iter() {
            instance.coin_ids.insert(id);
        }

        instance
    }
}

pub struct AssetsQuery<'a> {
    pub owner: &'a Address,
    pub assets: Option<HashSet<&'a AssetId>>,
    pub exclude: Option<&'a Exclude>,
    pub database: &'a Database,
    pub base_asset_id: &'a AssetId,
}

impl<'a> AssetsQuery<'a> {
    pub fn new(
        owner: &'a Address,
        assets: Option<HashSet<&'a AssetId>>,
        exclude: Option<&'a Exclude>,
        database: &'a Database,
        base_asset_id: &'a AssetId,
    ) -> Self {
        Self {
            owner,
            assets,
            exclude,
            database,
            base_asset_id,
        }
    }

    /// Returns the iterator over all valid(spendable, allowed by `exclude`) coins of the `owner`.
    ///
    /// # Note: The coins of different type are not grouped by the `asset_id`.
    // TODO: Optimize this by creating an index
    //  https://github.com/FuelLabs/fuel-core/issues/588
    pub fn coins(&self) -> impl Iterator<Item = StorageResult<CoinType>> + '_ {
        let coins_iter = self
            .database
            .owned_coins_ids(self.owner, None, IterDirection::Forward)
            .map(|id| id.map(CoinId::from))
            .filter_ok(|id| {
                if let Some(exclude) = self.exclude {
                    !exclude.coin_ids.contains(id)
                } else {
                    true
                }
            })
            .map(move |res| {
                res.map_err(StorageError::from).and_then(|id| {
                    let id = if let CoinId::Utxo(id) = id {
                        id
                    } else {
                        unreachable!("We've checked it above")
                    };
                    let coin = self.database.coin(id)?;

                    Ok(CoinType::Coin(coin))
                })
            })
            .filter_ok(|coin| {
                if let CoinType::Coin(coin) = coin {
                    self.assets
                        .as_ref()
                        .map(|assets| assets.contains(&coin.asset_id))
                        .unwrap_or(true)
                } else {
                    true
                }
            });

        let messages_iter = self
            .database
            .owned_message_ids(self.owner, None, IterDirection::Forward)
            .map(|id| id.map(CoinId::from))
            .filter_ok(|id| {
                if let Some(exclude) = self.exclude {
                    !exclude.coin_ids.contains(id)
                } else {
                    true
                }
            })
            .map(move |res| {
                res.and_then(|id| {
                    let id = if let CoinId::Message(id) = id {
                        id
                    } else {
                        unreachable!("We've checked it above")
                    };
                    let message = self.database.message(&id)?;
                    Ok(message)
                })
            })
            .filter_ok(|message| message.data.is_empty())
            .map(|result| {
                result.map(|message| {
                    CoinType::MessageCoin(
                        message
                            .try_into()
                            .expect("The checked above that message data is empty."),
                    )
                })
            });

        let c_iter = coins_iter.collect::<Vec<_>>();
        let predicate = self
            .assets
            .as_ref()
            .map(|assets| assets.contains(self.base_asset_id))
            .unwrap_or(true);
        dbg!(predicate);
        let m_iter = messages_iter.take_while(|_| predicate).collect::<Vec<_>>();
        dbg!(&c_iter);
        dbg!(&m_iter);
        let coins_iter = c_iter.into_iter();
        let messages_iter = m_iter.into_iter();

        coins_iter.chain(messages_iter)

        // let v = iter.collect::<Vec<_>>();
        // dbg!(&v);

        // v.into_iter()
    }
}

pub struct AssetQuery<'a> {
    pub owner: &'a Address,
    pub asset: &'a AssetSpendTarget,
    pub exclude: Option<&'a Exclude>,
    pub database: &'a Database,
    query: AssetsQuery<'a>,
}

impl<'a> AssetQuery<'a> {
    pub fn new(
        owner: &'a Address,
        asset: &'a AssetSpendTarget,
        base_asset_id: &'a AssetId,
        exclude: Option<&'a Exclude>,
        database: &'a Database,
    ) -> Self {
        let mut allowed = HashSet::new();
        allowed.insert(&asset.id);
        allowed.insert(&base_asset_id);
        Self {
            owner,
            asset,
            exclude,
            database,
            query: AssetsQuery::new(
                owner,
                Some(allowed),
                exclude,
                database,
                base_asset_id,
            ),
        }
    }

    /// Returns the iterator over all valid(spendable, allowed by `exclude`) coins of the `owner`
    /// for the `asset_id`.
    pub fn coins(&self) -> impl Iterator<Item = StorageResult<CoinType>> + '_ {
        self.query.coins()
    }
}
