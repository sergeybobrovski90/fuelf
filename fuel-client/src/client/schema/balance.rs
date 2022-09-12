use crate::client::{
    schema::{
        schema,
        Address,
        AssetId,
        PageInfo,
        U64,
    },
    PageDirection,
    PaginatedResult,
    PaginationRequest,
};

#[derive(cynic::FragmentArguments, Debug)]
pub struct BalanceArgs {
    pub owner: Address,
    pub asset_id: AssetId,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Query",
    argument_struct = "BalanceArgs"
)]
pub struct BalanceQuery {
    #[arguments(owner = &args.owner, asset_id = &args.asset_id)]
    pub balance: Balance,
}

#[derive(cynic::InputObject, Clone, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct BalanceFilterInput {
    /// Filter coins based on the `owner` field
    pub owner: Address,
}

#[derive(cynic::FragmentArguments, Debug)]
pub struct BalancesConnectionArgs {
    /// Filter coins based on a filter
    filter: BalanceFilterInput,
    /// Skip until coin id (forward pagination)
    pub after: Option<String>,
    /// Skip until coin id (backward pagination)
    pub before: Option<String>,
    /// Retrieve the first n coins in order (forward pagination)
    pub first: Option<i32>,
    /// Retrieve the last n coins in order (backward pagination).
    /// Can't be used at the same time as `first`.
    pub last: Option<i32>,
}

impl From<(Address, PaginationRequest<String>)> for BalancesConnectionArgs {
    fn from(r: (Address, PaginationRequest<String>)) -> Self {
        match r.1.direction {
            PageDirection::Forward => BalancesConnectionArgs {
                filter: BalanceFilterInput { owner: r.0 },
                after: r.1.cursor,
                before: None,
                first: Some(r.1.results as i32),
                last: None,
            },
            PageDirection::Backward => BalancesConnectionArgs {
                filter: BalanceFilterInput { owner: r.0 },
                after: None,
                before: r.1.cursor,
                first: None,
                last: Some(r.1.results as i32),
            },
        }
    }
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Query",
    argument_struct = "BalancesConnectionArgs"
)]
pub struct BalancesQuery {
    #[arguments(filter = &args.filter, after = &args.after, before = &args.before, first = &args.first, last = &args.last)]
    pub balances: BalanceConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct BalanceConnection {
    pub edges: Vec<BalanceEdge>,
    pub page_info: PageInfo,
}

impl From<BalanceConnection> for PaginatedResult<Balance, String> {
    fn from(conn: BalanceConnection) -> Self {
        PaginatedResult {
            has_next_page: conn.page_info.has_next_page,
            has_previous_page: conn.page_info.has_previous_page,
            cursor: conn.page_info.end_cursor,
            results: conn.edges.into_iter().map(|e| e.node).collect(),
        }
    }
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct BalanceEdge {
    pub cursor: String,
    pub node: Balance,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct Balance {
    pub owner: Address,
    pub amount: U64,
    pub asset_id: AssetId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_query_gql_output() {
        use cynic::QueryBuilder;
        let operation = BalanceQuery::build(BalanceArgs {
            owner: Address::default(),
            asset_id: AssetId::default(),
        });
        insta::assert_snapshot!(operation.query)
    }

    #[test]
    fn balances_connection_query_gql_output() {
        use cynic::QueryBuilder;
        let operation = BalancesQuery::build(BalancesConnectionArgs {
            filter: BalanceFilterInput {
                owner: Address::default(),
            },
            after: None,
            before: None,
            first: None,
            last: None,
        });
        insta::assert_snapshot!(operation.query)
    }
}
