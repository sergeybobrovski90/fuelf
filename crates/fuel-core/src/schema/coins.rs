use crate::{
    coins_query::{
        random_improve,
        SpendQuery,
    },
    fuel_core_graphql_api::{
        Config as GraphQLConfig,
        IntoApiResult,
    },
    graphql_api::service::Database,
    query::{
        asset_query::AssetSpendTarget,
        CoinQueryData,
    },
    schema::scalars::{
        Address,
        AssetId,
        Nonce,
        UtxoId,
        U32,
        U64,
    },
};
use async_graphql::{
    connection::{
        Connection,
        EmptyFields,
    },
    Context,
};
use fuel_core_types::{
    entities::{
        coins,
        coins::{
            coin::Coin as CoinModel,
            message_coin::MessageCoin as MessageCoinModel,
        },
    },
    fuel_tx,
};
use itertools::Itertools;

pub struct Coin(pub(crate) CoinModel);

#[async_graphql::Object]
impl Coin {
    async fn utxo_id(&self) -> UtxoId {
        self.0.utxo_id.into()
    }

    async fn owner(&self) -> Address {
        self.0.owner.into()
    }

    async fn amount(&self) -> U64 {
        self.0.amount.into()
    }

    async fn asset_id(&self) -> AssetId {
        self.0.asset_id.into()
    }

    async fn maturity(&self) -> U32 {
        self.0.maturity.into()
    }

    /// TxPointer - the height of the block this coin was created in
    async fn block_created(&self) -> U32 {
        u32::from(self.0.tx_pointer.block_height()).into()
    }

    /// TxPointer - the index of the transaction that created this coin
    async fn tx_created_idx(&self) -> U64 {
        u64::from(self.0.tx_pointer.tx_index()).into()
    }
}

pub struct MessageCoin(pub(crate) MessageCoinModel);

#[async_graphql::Object]
impl MessageCoin {
    async fn sender(&self) -> Address {
        self.0.sender.into()
    }

    async fn recipient(&self) -> Address {
        self.0.recipient.into()
    }

    async fn nonce(&self) -> Nonce {
        self.0.nonce.into()
    }

    async fn amount(&self) -> U64 {
        self.0.amount.into()
    }

    async fn asset_id(&self, ctx: &Context<'_>) -> AssetId {
        let config = ctx.data_unchecked::<GraphQLConfig>();
        let base_asset_id = *config.consensus_parameters.base_asset_id();
        base_asset_id.into()
    }

    async fn da_height(&self) -> U64 {
        self.0.da_height.0.into()
    }
}

/// The schema analog of the [`coins::CoinType`].
#[derive(async_graphql::Union)]
pub enum CoinType {
    /// The regular coins generated by the transaction output.
    Coin(Coin),
    /// The bridged coin from the DA layer.
    MessageCoin(MessageCoin),
}

#[derive(async_graphql::InputObject)]
struct CoinFilterInput {
    /// Returns coins owned by the `owner`.
    owner: Address,
    /// Returns coins only with `asset_id`.
    asset_id: Option<AssetId>,
}

#[derive(async_graphql::InputObject)]
pub struct SpendQueryElementInput {
    /// Identifier of the asset to spend.
    asset_id: AssetId,
    /// Target amount for the query.
    amount: U64,
    /// The maximum number of currencies for selection.
    max: Option<U32>,
}

#[derive(async_graphql::InputObject)]
pub struct ExcludeInput {
    /// Utxos to exclude from the selection.
    utxos: Vec<UtxoId>,
    /// Messages to exclude from the selection.
    messages: Vec<Nonce>,
}

#[derive(Default)]
pub struct CoinQuery;

#[async_graphql::Object]
impl CoinQuery {
    /// Gets the coin by `utxo_id`.
    async fn coin(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The ID of the coin")] utxo_id: UtxoId,
    ) -> async_graphql::Result<Option<Coin>> {
        let data: &Database = ctx.data_unchecked();
        data.coin(utxo_id.0).into_api_result()
    }

    /// Gets all unspent coins of some `owner` maybe filtered with by `asset_id` per page.
    async fn coins(
        &self,
        ctx: &Context<'_>,
        filter: CoinFilterInput,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> async_graphql::Result<Connection<UtxoId, Coin, EmptyFields, EmptyFields>> {
        let query: &Database = ctx.data_unchecked();
        crate::schema::query_pagination(after, before, first, last, |start, direction| {
            let owner: fuel_tx::Address = filter.owner.into();
            let coins = query
                .owned_coins(&owner, (*start).map(Into::into), direction)
                .filter_map(|result| {
                    if let (Ok(coin), Some(filter_asset_id)) = (&result, &filter.asset_id)
                    {
                        if coin.asset_id != filter_asset_id.0 {
                            return None
                        }
                    }

                    Some(result)
                })
                .map(|res| res.map(|coin| (coin.utxo_id.into(), coin.into())));

            Ok(coins)
        })
        .await
    }

    /// For each `query_per_asset`, get some spendable coins(of asset specified by the query) owned by
    /// `owner` that add up at least the query amount. The returned coins can be spent.
    /// The number of coins is optimized to prevent dust accumulation.
    ///
    /// The query supports excluding and maximum the number of coins.
    ///
    /// Returns:
    ///     The list of spendable coins per asset from the query. The length of the result is
    ///     the same as the length of `query_per_asset`. The ordering of assets and `query_per_asset`
    ///     is the same.
    async fn coins_to_spend(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The `Address` of the coins owner.")] owner: Address,
        #[graphql(desc = "\
            The list of requested assets` coins with asset ids, `target` amount the user wants \
            to reach, and the `max` number of coins in the selection. Several entries with the \
            same asset id are not allowed.")]
        query_per_asset: Vec<SpendQueryElementInput>,
        #[graphql(desc = "The excluded coins from the selection.")] excluded_ids: Option<
            ExcludeInput,
        >,
    ) -> async_graphql::Result<Vec<Vec<CoinType>>> {
        let config = ctx.data_unchecked::<GraphQLConfig>();

        let owner: fuel_tx::Address = owner.0;
        let query_per_asset = query_per_asset
            .into_iter()
            .map(|e| {
                AssetSpendTarget::new(
                    e.asset_id.0,
                    e.amount.0,
                    e.max.map(|max| max.0 as usize).unwrap_or(usize::MAX),
                )
            })
            .collect_vec();
        let excluded_ids: Option<Vec<_>> = excluded_ids.map(|exclude| {
            let utxos = exclude
                .utxos
                .into_iter()
                .map(|utxo| coins::CoinId::Utxo(utxo.into()));
            let messages = exclude
                .messages
                .into_iter()
                .map(|message| coins::CoinId::Message(message.into()));
            utxos.chain(messages).collect()
        });

        let base_asset_id = config.consensus_parameters.base_asset_id();
        let spend_query =
            SpendQuery::new(owner, &query_per_asset, excluded_ids, *base_asset_id)?;

        let db = ctx.data_unchecked::<Database>();

        let coins = random_improve(db, &spend_query)?
            .into_iter()
            .map(|coins| {
                coins
                    .into_iter()
                    .map(|coin| match coin {
                        coins::CoinType::Coin(coin) => CoinType::Coin(coin.into()),
                        coins::CoinType::MessageCoin(coin) => {
                            CoinType::MessageCoin(coin.into())
                        }
                    })
                    .collect_vec()
            })
            .collect();

        Ok(coins)
    }
}

impl From<CoinModel> for Coin {
    fn from(value: CoinModel) -> Self {
        Coin(value)
    }
}

impl From<MessageCoinModel> for MessageCoin {
    fn from(value: MessageCoinModel) -> Self {
        MessageCoin(value)
    }
}

impl From<coins::CoinType> for CoinType {
    fn from(value: coins::CoinType) -> Self {
        match value {
            coins::CoinType::Coin(coin) => CoinType::Coin(coin.into()),
            coins::CoinType::MessageCoin(coin) => CoinType::MessageCoin(coin.into()),
        }
    }
}
