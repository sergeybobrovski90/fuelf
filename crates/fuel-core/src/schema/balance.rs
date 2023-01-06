use crate::{
    database::resource::AssetsQuery,
    fuel_core_graphql_api::service::Database,
    query::{
        BalanceQueryContext,
        BalanceQueryData,
    },
    schema::scalars::{
        Address,
        AssetId,
        U64,
    },
    state::IterDirection,
};
use async_graphql::{
    connection::{
        Connection,
        EmptyFields,
    },
    Context,
    InputObject,
    Object,
};
use fuel_core_types::fuel_types;
use itertools::Itertools;
use std::{
    cmp::Ordering,
    collections::HashMap,
};

pub struct Balance {
    owner: fuel_types::Address,
    amount: u64,
    asset_id: fuel_types::AssetId,
}

#[Object]
impl Balance {
    async fn owner(&self) -> Address {
        self.owner.into()
    }

    async fn amount(&self) -> U64 {
        self.amount.into()
    }

    async fn asset_id(&self) -> AssetId {
        self.asset_id.into()
    }
}

#[derive(InputObject)]
struct BalanceFilterInput {
    /// Filter coins based on the `owner` field
    owner: Address,
}

#[derive(Default)]
pub struct BalanceQuery;

#[Object]
impl BalanceQuery {
    async fn balance(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "address of the owner")] owner: Address,
        #[graphql(desc = "asset_id of the coin")] asset_id: AssetId,
    ) -> async_graphql::Result<Balance> {
        let data = BalanceQueryContext(ctx.data_unchecked());
        let balance = data.balance(owner.0, asset_id.0)?;

        Ok(Balance {
            owner: balance.0.into(),
            amount: balance.1,
            asset_id: balance.2,
        })
    }

    // TODO: We can't paginate over `AssetId` because it is not unique.
    //  It should be replaced with `UtxoId`.
    async fn balances(
        &self,
        ctx: &Context<'_>,
        filter: BalanceFilterInput,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> async_graphql::Result<Connection<AssetId, Balance, EmptyFields, EmptyFields>>
    {
        let db = ctx.data_unchecked::<Database>();
        crate::schema::query_pagination(after, before, first, last, |_, direction| {
            let owner = filter.owner.into();

            let mut amounts_per_asset = HashMap::new();

            for resource in AssetsQuery::new(&owner, None, None, db).unspent_resources() {
                let resource = resource?;
                *amounts_per_asset.entry(*resource.asset_id()).or_default() +=
                    resource.amount();
            }

            let mut balances = amounts_per_asset
                .into_iter()
                .map(|(asset_id, amount)| Balance {
                    owner,
                    amount,
                    asset_id,
                })
                .collect_vec();
            balances.sort_by(|l, r| {
                if l.asset_id < r.asset_id {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            });
            if direction == IterDirection::Reverse {
                balances.reverse();
            }

            let balances = balances
                .into_iter()
                .map(|balance| Ok((balance.asset_id.into(), balance)));

            Ok(balances)
        })
        .await
    }
}
