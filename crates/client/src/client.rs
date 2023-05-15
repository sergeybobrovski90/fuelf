use crate::client::schema::{
    block::BlockByHeightArgs,
    coins::{
        ExcludeInput,
        SpendQueryElementInput,
    },
    contract::ContractBalanceQueryArgs,
    tx::DryRunArg,
    Tai64Timestamp,
};
use anyhow::Context;
#[cfg(feature = "subscriptions")]
use cynic::StreamingOperation;
use cynic::{
    http::ReqwestExt,
    GraphQlResponse,
    Id,
    MutationBuilder,
    Operation,
    QueryBuilder,
};
use fuel_core_types::{
    fuel_asm::{
        Instruction,
        RegisterId,
        Word,
    },
    fuel_tx::{
        Receipt,
        Transaction,
    },
    fuel_types,
    fuel_types::{
        bytes::SerializableVec,
        BlockHeight,
    },
};
#[cfg(feature = "subscriptions")]
use futures::StreamExt;
use itertools::Itertools;
pub use pagination::{
    PageDirection,
    PaginatedResult,
    PaginationRequest,
};
use reqwest::cookie::CookieStore;
use schema::{
    balance::BalanceArgs,
    block::BlockByIdArgs,
    coins::CoinByIdArgs,
    contract::ContractByIdArgs,
    tx::{
        TxArg,
        TxIdArgs,
    },
    Bytes,
    ContinueTx,
    ContinueTxArgs,
    ConversionError,
    HexString,
    IdArg,
    MemoryArgs,
    RegisterArgs,
    RunResult,
    SetBreakpoint,
    SetBreakpointArgs,
    SetSingleStepping,
    SetSingleSteppingArgs,
    StartTx,
    StartTxArgs,
    U64,
};
#[cfg(feature = "subscriptions")]
use std::future;
use std::{
    convert::TryInto,
    io::{
        self,
        ErrorKind,
    },
    net,
    ops::Deref,
    str::{
        self,
        FromStr,
    },
    sync::Arc,
};
use tai64::Tai64;
use tracing as _;
use types::{
    TransactionResponse,
    TransactionStatus,
};

use self::schema::{
    block::ProduceBlockArgs,
    message::MessageProofArgs,
};

mod hex_formatted;
use hex_formatted::HexFormatted;

mod pagination;
pub mod schema;
pub mod types;

#[derive(Debug, Clone)]
pub struct FuelClient {
    client: reqwest::Client,
    cookie: Arc<reqwest::cookie::Jar>,
    url: reqwest::Url,
}

impl FromStr for FuelClient {
    type Err = anyhow::Error;

    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let mut raw_url = str.to_string();
        if !raw_url.starts_with("http") {
            raw_url = format!("http://{raw_url}");
        }

        let mut url = reqwest::Url::parse(&raw_url)
            .with_context(|| format!("Invalid fuel-core URL: {str}"))?;
        url.set_path("/graphql");
        let cookie = Arc::new(reqwest::cookie::Jar::default());
        let client = reqwest::Client::builder()
            .cookie_provider(cookie.clone())
            .build()?;
        Ok(Self {
            client,
            cookie,
            url,
        })
    }
}

impl<S> From<S> for FuelClient
where
    S: Into<net::SocketAddr>,
{
    fn from(socket: S) -> Self {
        format!("http://{}", socket.into())
            .as_str()
            .parse()
            .unwrap()
    }
}

pub fn from_strings_errors_to_std_error(errors: Vec<String>) -> io::Error {
    let e = errors
        .into_iter()
        .fold(String::from("Response errors"), |mut s, e| {
            s.push_str("; ");
            s.push_str(e.as_str());
            s
        });
    io::Error::new(io::ErrorKind::Other, e)
}

impl FuelClient {
    pub fn new(url: impl AsRef<str>) -> anyhow::Result<Self> {
        Self::from_str(url.as_ref())
    }

    async fn query<ResponseData, Vars>(
        &self,
        q: Operation<ResponseData, Vars>,
    ) -> io::Result<ResponseData>
    where
        Vars: serde::Serialize,
        ResponseData: serde::de::DeserializeOwned + 'static,
    {
        let response = self
            .client
            .post(self.url.clone())
            .run_graphql(q)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Self::decode_response(response)
    }

    fn decode_response<R>(response: GraphQlResponse<R>) -> io::Result<R>
    where
        R: serde::de::DeserializeOwned + 'static,
    {
        match (response.data, response.errors) {
            (Some(d), _) => Ok(d),
            (_, Some(e)) => Err(from_strings_errors_to_std_error(
                e.into_iter().map(|e| e.message).collect(),
            )),
            _ => Err(io::Error::new(io::ErrorKind::Other, "Invalid response")),
        }
    }

    #[tracing::instrument(skip_all)]
    #[cfg(feature = "subscriptions")]
    async fn subscribe<ResponseData, Vars>(
        &self,
        q: StreamingOperation<ResponseData, Vars>,
    ) -> io::Result<impl futures::Stream<Item = io::Result<ResponseData>>>
    where
        Vars: serde::Serialize,
        ResponseData: serde::de::DeserializeOwned + 'static,
    {
        use eventsource_client as es;
        use hyper_rustls as _;
        let mut url = self.url.clone();
        url.set_path("/graphql-sub");
        let json_query = serde_json::to_string(&q)?;
        let mut client_builder = es::ClientBuilder::for_url(url.as_str())
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("Failed to start client {e:?}"),
                )
            })?
            .body(json_query)
            .method("POST".to_string())
            .header("content-type", "application/json")
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("Failed to add header to client {e:?}"),
                )
            })?;

        if let Some(value) = self.cookie.deref().cookies(&self.url) {
            let value = value.to_str().map_err(|e| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("Unable convert header value to string {e:?}"),
                )
            })?;
            client_builder = client_builder
                .header(reqwest::header::COOKIE.as_str(), value)
                .map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("Failed to add header from `reqwest` to client {e:?}"),
                    )
                })?;
        }

        let client =
            client_builder.build_with_conn(es::HttpsConnector::with_webpki_roots());

        let mut last = None;

        let stream = es::Client::stream(&client)
            .take_while(|result| {
                futures::future::ready(!matches!(result, Err(es::Error::Eof)))
            })
            .filter_map(move |result| {
                tracing::debug!("Got result: {result:?}");
                let r = match result {
                    Ok(es::SSE::Event(es::Event { data, .. })) => {
                        match serde_json::from_str::<GraphQlResponse<ResponseData>>(&data)
                        {
                            Ok(resp) => {
                                match Self::decode_response(resp) {
                                    Ok(resp) => {
                                        match last.replace(data) {
                                            // Remove duplicates
                                            Some(l)
                                                if l == *last.as_ref().expect(
                                                    "Safe because of the replace above",
                                                ) =>
                                            {
                                                None
                                            }
                                            _ => Some(Ok(resp)),
                                        }
                                    }
                                    Err(e) => Some(Err(io::Error::new(
                                        io::ErrorKind::Other,
                                        format!("Decode error: {e:?}"),
                                    ))),
                                }
                            }
                            Err(e) => Some(Err(io::Error::new(
                                io::ErrorKind::Other,
                                format!("Json error: {e:?}"),
                            ))),
                        }
                    }
                    Ok(_) => None,
                    Err(e) => Some(Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Graphql error: {e:?}"),
                    ))),
                };
                futures::future::ready(r)
            });

        Ok(stream)
    }

    pub async fn health(&self) -> io::Result<bool> {
        let query = schema::Health::build(());
        self.query(query).await.map(|r| r.health)
    }

    pub async fn node_info(&self) -> io::Result<types::NodeInfo> {
        let query = schema::node_info::QueryNodeInfo::build(());
        self.query(query).await.map(|r| r.node_info.into())
    }

    pub async fn chain_info(&self) -> io::Result<types::ChainInfo> {
        let query = schema::chain::ChainQuery::build(());
        self.query(query).await.map(|r| r.chain.into())
    }

    /// Default dry run, matching the exact configuration as the node
    pub async fn dry_run(&self, tx: &Transaction) -> io::Result<Vec<Receipt>> {
        self.dry_run_opt(tx, None).await
    }

    /// Dry run with options to override the node behavior
    pub async fn dry_run_opt(
        &self,
        tx: &Transaction,
        // Disable utxo input checks (exists, unspent, and valid signature)
        utxo_validation: Option<bool>,
    ) -> io::Result<Vec<Receipt>> {
        let tx = tx.clone().to_bytes();
        let query = schema::tx::DryRun::build(DryRunArg {
            tx: HexString(Bytes(tx)),
            utxo_validation,
        });
        let receipts = self.query(query).await.map(|r| r.dry_run)?;
        receipts
            .into_iter()
            .map(|receipt| receipt.try_into().map_err(Into::into))
            .collect()
    }

    pub async fn submit(
        &self,
        tx: &Transaction,
    ) -> io::Result<types::scalars::TransactionId> {
        let tx = tx.clone().to_bytes();
        let query = schema::tx::Submit::build(TxArg {
            tx: HexString(Bytes(tx)),
        });

        let id = self.query(query).await.map(|r| r.submit)?.id.into();
        Ok(id)
    }

    #[cfg(feature = "subscriptions")]
    /// Submit the transaction and wait for it to be included into a block.
    ///
    /// This will wait forever if needed, so consider wrapping this call
    /// with a `tokio::time::timeout`.
    pub async fn submit_and_await_commit(
        &self,
        tx: &Transaction,
    ) -> io::Result<TransactionStatus> {
        let tx_id = self.submit(tx).await?;
        self.await_transaction_commit(&tx_id.to_string()).await
    }

    pub async fn start_session(&self) -> io::Result<String> {
        let query = schema::StartSession::build(());

        self.query(query)
            .await
            .map(|r| r.start_session.into_inner())
    }

    pub async fn end_session(&self, id: &str) -> io::Result<bool> {
        let query = schema::EndSession::build(IdArg { id: id.into() });

        self.query(query).await.map(|r| r.end_session)
    }

    pub async fn reset(&self, id: &str) -> io::Result<bool> {
        let query = schema::Reset::build(IdArg { id: id.into() });

        self.query(query).await.map(|r| r.reset)
    }

    pub async fn execute(&self, id: &str, op: &Instruction) -> io::Result<bool> {
        let op = serde_json::to_string(op)?;
        let query = schema::Execute::build(schema::ExecuteArgs { id: id.into(), op });

        self.query(query).await.map(|r| r.execute)
    }

    pub async fn register(&self, id: &str, register: RegisterId) -> io::Result<Word> {
        let query = schema::Register::build(RegisterArgs {
            id: id.into(),
            register: register.into(),
        });

        Ok(self.query(query).await?.register.0 as Word)
    }

    pub async fn memory(
        &self,
        id: &str,
        start: usize,
        size: usize,
    ) -> io::Result<Vec<u8>> {
        let query = schema::Memory::build(MemoryArgs {
            id: id.into(),
            start: start.into(),
            size: size.into(),
        });

        let memory = self.query(query).await?.memory;

        Ok(serde_json::from_str(memory.as_str())?)
    }

    pub async fn set_breakpoint(
        &self,
        session_id: &str,
        contract: fuel_types::ContractId,
        pc: u64,
    ) -> io::Result<()> {
        let operation = SetBreakpoint::build(SetBreakpointArgs {
            id: Id::new(session_id),
            bp: schema::Breakpoint {
                contract: contract.into(),
                pc: U64(pc),
            },
        });

        let response = self.query(operation).await?;
        assert!(
            response.set_breakpoint,
            "Setting breakpoint returned invalid reply"
        );
        Ok(())
    }

    pub async fn set_single_stepping(
        &self,
        session_id: &str,
        enable: bool,
    ) -> io::Result<()> {
        let operation = SetSingleStepping::build(SetSingleSteppingArgs {
            id: Id::new(session_id),
            enable,
        });
        self.query(operation).await?;
        Ok(())
    }

    pub async fn start_tx(
        &self,
        session_id: &str,
        tx: &Transaction,
    ) -> io::Result<RunResult> {
        let operation = StartTx::build(StartTxArgs {
            id: Id::new(session_id),
            tx: serde_json::to_string(tx).expect("Couldn't serialize tx to json"),
        });
        let response = self.query(operation).await?.start_tx;
        Ok(response)
    }

    pub async fn continue_tx(&self, session_id: &str) -> io::Result<RunResult> {
        let operation = ContinueTx::build(ContinueTxArgs {
            id: Id::new(session_id),
        });
        let response = self.query(operation).await?.continue_tx;
        Ok(response)
    }

    pub async fn transaction(&self, id: &str) -> io::Result<Option<TransactionResponse>> {
        let query = schema::tx::TransactionQuery::build(TxIdArgs { id: id.parse()? });

        let transaction = self.query(query).await?.transaction;

        Ok(transaction.map(|tx| tx.try_into()).transpose()?)
    }

    /// Get the status of a transaction
    pub async fn transaction_status(&self, id: &str) -> io::Result<TransactionStatus> {
        let query = schema::tx::TransactionQuery::build(TxIdArgs { id: id.parse()? });

        let tx = self.query(query).await?.transaction.ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                format!("status not found for transaction {id} "),
            )
        })?;

        let status = tx
            .status
            .ok_or_else(|| {
                io::Error::new(
                    ErrorKind::NotFound,
                    format!("status not found for transaction {id}"),
                )
            })?
            .try_into()?;
        Ok(status)
    }

    #[tracing::instrument(skip(self), level = "debug")]
    #[cfg(feature = "subscriptions")]
    /// Subscribe to the status of a transaction
    pub async fn subscribe_transaction_status(
        &self,
        id: &str,
    ) -> io::Result<impl futures::Stream<Item = io::Result<TransactionStatus>>> {
        use cynic::SubscriptionBuilder;
        let s = schema::tx::StatusChangeSubscription::build(TxIdArgs { id: id.parse()? });

        tracing::debug!("subscribing");
        let stream = self.subscribe(s).await?.map(|tx| {
            tracing::debug!("received {tx:?}");
            let tx = tx?;
            let status = tx.status_change.try_into()?;
            Ok(status)
        });

        Ok(stream)
    }

    #[cfg(feature = "subscriptions")]
    /// Awaits for the transaction to be committed into a block
    ///
    /// This will wait forever if needed, so consider wrapping this call
    /// with a `tokio::time::timeout`.
    pub async fn await_transaction_commit(
        &self,
        id: &str,
    ) -> io::Result<TransactionStatus> {
        // skip until we've reached a final status and then stop consuming the stream
        // to avoid an EOF which the eventsource client considers as an error.
        let status_result = self
            .subscribe_transaction_status(id)
            .await?
            .skip_while(|status| {
                future::ready(matches!(status, Ok(TransactionStatus::Submitted { .. })))
            })
            .next()
            .await;

        if let Some(Ok(status)) = status_result {
            Ok(status)
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Failed to get status for transaction {status_result:?}"),
            ))
        }
    }

    /// returns a paginated set of transactions sorted by block height
    pub async fn transactions(
        &self,
        request: PaginationRequest<String>,
    ) -> io::Result<PaginatedResult<TransactionResponse, String>> {
        let query = schema::tx::TransactionsQuery::build(request.into());
        let transactions = self.query(query).await?.transactions.try_into()?;
        Ok(transactions)
    }

    /// Returns a paginated set of transactions associated with a txo owner address.
    pub async fn transactions_by_owner(
        &self,
        owner: &str,
        request: PaginationRequest<String>,
    ) -> io::Result<PaginatedResult<TransactionResponse, String>> {
        let owner: schema::Address = owner.parse()?;
        let query = schema::tx::TransactionsByOwnerQuery::build((owner, request).into());

        let transactions = self.query(query).await?.transactions_by_owner.try_into()?;
        Ok(transactions)
    }

    pub async fn receipts(&self, id: &str) -> io::Result<Option<Vec<Receipt>>> {
        let query = schema::tx::TransactionQuery::build(TxIdArgs { id: id.parse()? });

        let tx = self.query(query).await?.transaction.ok_or_else(|| {
            io::Error::new(ErrorKind::NotFound, format!("transaction {id} not found"))
        })?;

        let receipts = tx
            .receipts
            .map(|vec| {
                let vec: Result<Vec<Receipt>, ConversionError> =
                    vec.into_iter().map(TryInto::<Receipt>::try_into).collect();
                vec
            })
            .transpose()?;

        Ok(receipts)
    }

    pub async fn produce_blocks(
        &self,
        blocks_to_produce: u64,
        start_timestamp: Option<u64>,
    ) -> io::Result<BlockHeight> {
        let query = schema::block::BlockMutation::build(ProduceBlockArgs {
            blocks_to_produce: blocks_to_produce.into(),
            start_timestamp: start_timestamp
                .map(|timestamp| Tai64Timestamp::from(Tai64(timestamp))),
        });

        let new_height = self.query(query).await?.produce_blocks;

        Ok(new_height.into())
    }

    pub async fn block(&self, id: &str) -> io::Result<Option<types::Block>> {
        let query = schema::block::BlockByIdQuery::build(BlockByIdArgs {
            id: Some(id.parse()?),
        });

        let block = self.query(query).await?.block.map(Into::into);

        Ok(block)
    }

    pub async fn block_by_height(&self, height: u64) -> io::Result<Option<types::Block>> {
        let query = schema::block::BlockByHeightQuery::build(BlockByHeightArgs {
            height: Some(U64(height)),
        });

        let block = self.query(query).await?.block.map(Into::into);

        Ok(block)
    }

    /// Retrieve multiple blocks
    pub async fn blocks(
        &self,
        request: PaginationRequest<String>,
    ) -> io::Result<PaginatedResult<types::Block, String>> {
        let query = schema::block::BlocksQuery::build(request.into());

        let blocks = self.query(query).await?.blocks.into();

        Ok(blocks)
    }

    pub async fn coin(&self, id: &str) -> io::Result<Option<types::Coin>> {
        let query = schema::coins::CoinByIdQuery::build(CoinByIdArgs {
            utxo_id: id.parse()?,
        });
        let coin = self.query(query).await?.coin.map(Into::into);
        Ok(coin)
    }

    /// Retrieve a page of coins by their owner
    pub async fn coins(
        &self,
        owner: &str,
        asset_id: Option<&str>,
        request: PaginationRequest<String>,
    ) -> io::Result<PaginatedResult<types::Coin, String>> {
        let owner: schema::Address = owner.parse()?;
        let asset_id: schema::AssetId = match asset_id {
            Some(asset_id) => asset_id.parse()?,
            None => schema::AssetId::default(),
        };
        let query = schema::coins::CoinsQuery::build((owner, asset_id, request).into());

        let coins = self.query(query).await?.coins.into();
        Ok(coins)
    }

    /// Retrieve coins to spend in a transaction
    pub async fn coins_to_spend(
        &self,
        owner: &str,
        spend_query: Vec<(&str, u64, Option<u64>)>,
        // (Utxos, Messages Nonce)
        excluded_ids: Option<(Vec<&str>, Vec<&str>)>,
    ) -> io::Result<Vec<Vec<types::CoinType>>> {
        let owner: schema::Address = owner.parse()?;
        let spend_query: Vec<SpendQueryElementInput> = spend_query
            .iter()
            .map(|(asset_id, amount, max)| -> Result<_, ConversionError> {
                Ok(SpendQueryElementInput {
                    asset_id: asset_id.parse()?,
                    amount: (*amount).into(),
                    max: (*max).map(|max| max.into()),
                })
            })
            .try_collect()?;
        let excluded_ids: Option<ExcludeInput> =
            excluded_ids.map(ExcludeInput::from_tuple).transpose()?;
        let query = schema::coins::CoinsToSpendQuery::build(
            (owner, spend_query, excluded_ids).into(),
        );

        let coins_per_asset = self
            .query(query)
            .await?
            .coins_to_spend
            .iter()
            .map(|v| v.iter().cloned().map(Into::into).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        Ok(coins_per_asset)
    }

    pub async fn contract(&self, id: &str) -> io::Result<Option<types::Contract>> {
        let query = schema::contract::ContractByIdQuery::build(ContractByIdArgs {
            id: id.parse()?,
        });
        let contract = self.query(query).await?.contract.map(Into::into);
        Ok(contract)
    }

    pub async fn contract_balance(
        &self,
        id: &str,
        asset: Option<&str>,
    ) -> io::Result<u64> {
        let asset_id: schema::AssetId = match asset {
            Some(asset) => asset.parse()?,
            None => schema::AssetId::default(),
        };

        let query =
            schema::contract::ContractBalanceQuery::build(ContractBalanceQueryArgs {
                id: id.parse()?,
                asset: asset_id,
            });

        let balance: types::ContractBalance =
            self.query(query).await.unwrap().contract_balance.into();
        Ok(balance.amount)
    }

    pub async fn balance(&self, owner: &str, asset_id: Option<&str>) -> io::Result<u64> {
        let owner: schema::Address = owner.parse()?;
        let asset_id: schema::AssetId = match asset_id {
            Some(asset_id) => asset_id.parse()?,
            None => schema::AssetId::default(),
        };
        let query = schema::balance::BalanceQuery::build(BalanceArgs { owner, asset_id });
        let balance: types::Balance = self.query(query).await?.balance.into();
        Ok(balance.amount)
    }

    // Retrieve a page of balances by their owner
    pub async fn balances(
        &self,
        owner: &str,
        request: PaginationRequest<String>,
    ) -> io::Result<PaginatedResult<types::Balance, String>> {
        let owner: schema::Address = owner.parse()?;
        let query = schema::balance::BalancesQuery::build((owner, request).into());

        let balances = self.query(query).await?.balances.into();
        Ok(balances)
    }

    pub async fn contract_balances(
        &self,
        contract: &str,
        request: PaginationRequest<String>,
    ) -> io::Result<PaginatedResult<types::ContractBalance, String>> {
        let contract_id: schema::ContractId = contract.parse()?;
        let query =
            schema::contract::ContractBalancesQuery::build((contract_id, request).into());

        let balances = self.query(query).await?.contract_balances.into();

        Ok(balances)
    }

    pub async fn messages(
        &self,
        owner: Option<&str>,
        request: PaginationRequest<String>,
    ) -> io::Result<PaginatedResult<types::Message, String>> {
        let owner: Option<schema::Address> =
            owner.map(|owner| owner.parse()).transpose()?;
        let query = schema::message::OwnedMessageQuery::build((owner, request).into());

        let messages = self.query(query).await?.messages.into();

        Ok(messages)
    }

    /// Request a merkle proof of an output message.
    pub async fn message_proof(
        &self,
        transaction_id: &str,
        message_id: &str,
        commit_block_id: Option<&str>,
        commit_block_height: Option<BlockHeight>,
    ) -> io::Result<Option<types::MessageProof>> {
        let transaction_id: schema::TransactionId = transaction_id.parse()?;
        let message_id: schema::MessageId = message_id.parse()?;
        let commit_block_id: Option<schema::BlockId> = commit_block_id
            .map(|commit_block_id| commit_block_id.parse())
            .transpose()?;
        let commit_block_height = commit_block_height.map(Into::into);
        let query = schema::message::MessageProofQuery::build(MessageProofArgs {
            transaction_id,
            message_id,
            commit_block_id,
            commit_block_height,
        });

        let proof = self.query(query).await?.message_proof.map(Into::into);

        Ok(proof)
    }
}

#[cfg(any(test, feature = "test-helpers"))]
impl FuelClient {
    pub async fn transparent_transaction(
        &self,
        id: &str,
    ) -> io::Result<Option<Transaction>> {
        let query = schema::tx::TransactionQuery::build(TxIdArgs { id: id.parse()? });

        let transaction = self.query(query).await?.transaction;

        Ok(transaction.map(|tx| tx.try_into()).transpose()?)
    }
}
