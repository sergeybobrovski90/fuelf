use async_trait::async_trait;
use bytes::Bytes;
use ethers_core::types::{
    Block, BlockId, Bytes as EthersBytes, Filter, Log, TxHash, H160, H256, U256, U64,
};
use ethers_providers::{
    FilterKind, FilterWatcher, JsonRpcClient, Middleware, MockProvider, Provider, ProviderError,
    SyncingStatus,
};
use parking_lot::Mutex;
use serde::{de::DeserializeOwned, Serialize};
use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{fmt::Debug, str::FromStr};
use thiserror::Error;

type TriggerHandler = dyn FnMut(&mut MockData, TriggerType) -> bool + Send + Sync;

#[derive(Clone)]
pub struct MockMiddleware {
    pub inner: Box<Option<Provider<MockMiddleware>>>,
    pub data: Arc<Mutex<MockData>>,
    pub triggers: Arc<Mutex<Vec<Box<TriggerHandler>>>>,
    pub triggers_index: Arc<AtomicUsize>,
    //_phantom: PhantomData<&'a Self>,
}

impl fmt::Debug for MockMiddleware {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockMiddleware")
            .field("data", &self.data)
            .finish()
    }
}

#[derive(Debug)]
pub struct MockData {
    pub is_syncing: SyncingStatus,
    pub best_block: Block<TxHash>,
    pub logs_batch: Vec<Vec<Log>>,
    pub logs_batch_index: usize,
}

impl Default for MockData {
    fn default() -> Self {
        let mut best_block = Block::default();
        best_block.hash = Some(
            H256::from_str("0xa1ea3121940930f7e7b54506d80717f14c5163807951624c36354202a8bffda6")
                .unwrap(),
        );
        best_block.number = Some(U64::from(20i32));
        MockData {
            best_block,
            is_syncing: SyncingStatus::IsFalse,
            logs_batch: Vec::new(),
            logs_batch_index: 0,
        }
    }
}

impl MockMiddleware {
    /// Instantiates the nonce manager with a 0 nonce. The `address` should be the
    /// address which you'll be sending transactions from
    pub fn new() -> Self {
        let mut s = Self {
            inner: Box::new(None),
            data: Arc::new(Mutex::new(MockData::default())),
            triggers: Arc::new(Mutex::new(Vec::new())),
            triggers_index: Arc::new(AtomicUsize::new(0)),
            //_phantom: PhantomData::default(),
        };
        let sc = s.clone();
        s.inner = Box::new(Some(Provider::new(sc)));
        s
    }

    pub async fn insert_log_batch(&mut self, logs: Vec<Log>) {
        self.data.lock().logs_batch.push(logs)
    }

    fn trigger(&self, trigger_type: TriggerType) {
        if let Some(trigger) = self
            .triggers
            .lock()
            .get_mut(self.triggers_index.load(Ordering::SeqCst))
        {
            let mut mock_data = self.data.lock();
            if trigger(&mut mock_data, trigger_type) {
                self.triggers_index.fetch_add(1, Ordering::SeqCst);
            }
        } else {
            assert!(true, "Badly structured test in Relayer MockMiddleware");
        }
    }
}

#[derive(Error, Debug)]
/// Thrown when an error happens at the Nonce Manager
pub enum MockMiddlewareError {
    /// Thrown when the internal middleware errors
    #[error("Test")]
    MiddlewareError(),
    #[error("Internal error")]
    Internal,
}

// impl<M: Middleware> FromErr<M::Error> for MockMiddlewareError {
//     fn from(src: M::Error) -> Self {
//         Self::MiddlewareError(src)
//     }
// }

#[derive(Debug)]
pub enum TriggerType {
    Syncing,
    GetBlockNumber,
    GetLogs(Filter),
    GetBlock(BlockId),
    GetFilterChanges(U256),
}

#[async_trait]
impl JsonRpcClient for MockMiddleware {
    /// A JSON-RPC Error
    type Error = ProviderError;

    /// Sends a request with the provided JSON-RPC and parameters serialized as JSON
    async fn request<T, R>(&self, method: &str, params: T) -> Result<R, Self::Error>
    where
        T: Debug + Serialize + Send + Sync,
        R: DeserializeOwned,
    {
        //if method == 'eth_getFilterChanges' {
            let block_id = U256::zero();
            self.trigger(TriggerType::GetFilterChanges(block_id.clone()));
    
            let data = self.data.lock();
            Ok(
                if let Some(logs) = data.logs_batch.get(data.logs_batch_index) {
                    let mut ret_logs = Vec::new();
                    for log in logs {
                        // let log = serde_json::to_value(&log).map_err(|e| Self::Error::SerdeJson(e))?;
                        // let res: R =
                        //    ;
                        ret_logs.push(log);
                    }
                    //ret_logs
                    let res = serde_json::to_value(&ret_logs).map_err(|e| Self::Error::SerdeJson(e))?;
                    let res: R = serde_json::from_value(res).map_err(|e| Self::Error::SerdeJson(e))?;
                    res
                } else {
                    let ret : Vec<Log> = Vec::new();
                    let res = serde_json::to_value(ret)?;
                    let res: R = serde_json::from_value(res).map_err(|e| Self::Error::SerdeJson(e))?;
                    res
                }
            )
        //}
    }
}

/*
WHAT DO I NEED FOR RELAYER FROM PROVIDER:
* syncing API
* get_block_number API
* get_logs API.
* .watch() API for logs with filter. Impl LogStream
    * LogsWatcher only uses .next()
* get_block API using only HASH
*/


#[async_trait]
impl Middleware for MockMiddleware {
    type Error = ProviderError;
    type Provider = Self;
    type Inner = Self;

    fn inner(&self) -> &Self::Inner {
        unreachable!("There is no inner provider here")
    }

    /// Needs for initial sync of relayer
    async fn syncing(&self) -> Result<SyncingStatus, Self::Error> {
        self.trigger(TriggerType::Syncing);
        Ok(self.data.lock().is_syncing.clone())
    }

    /// Used in initial sync to get current best eth block
    async fn get_block_number(&self) -> Result<U64, Self::Error> {
        self.trigger(TriggerType::GetBlockNumber);
        Ok(self.data.lock().best_block.number.unwrap())
    }

    /// used for initial sync to get logs of already finalized diffs
    async fn get_logs(&self, filter: &Filter) -> Result<Vec<Log>, Self::Error> {
        self.trigger(TriggerType::GetLogs(filter.clone()));
        Ok(Vec::new())
    }

    /// used for initial sync to get block hash. Other fields can be ignored.
    async fn get_block<T: Into<BlockId> + Send + Sync>(
        &self,
        block_hash_or_number: T,
    ) -> Result<Option<Block<TxHash>>, Self::Error> {
        let block_id = block_hash_or_number.into();
        self.trigger(TriggerType::GetBlock(block_id.clone()));
        // TODO change
        Ok(Some(self.data.lock().best_block.clone()))
    }

    /// only thing used FilterWatcher
    async fn get_filter_changes<T, R>(&self, id: T) -> Result<Vec<R>, Self::Error>
    where
        T: Into<U256> + Send + Sync,
        R: Serialize + DeserializeOwned + Send + Sync + Debug,
    {
        let block_id = id.into();
        self.trigger(TriggerType::GetFilterChanges(block_id.clone()));

        let data = self.data.lock();
        Ok(
            if let Some(logs) = data.logs_batch.get(data.logs_batch_index) {
                let mut ret_logs = Vec::new();
                for log in logs {
                    let log = serde_json::to_value(&log).map_err(|e| Self::Error::SerdeJson(e))?;
                    let res: R =
                        serde_json::from_value(log).map_err(|e| Self::Error::SerdeJson(e))?;
                    ret_logs.push(res);
                }
                ret_logs
            } else {
                Vec::new()
            },
        )
    }

    async fn watch<'b>(
        &'b self,
        filter: &Filter,
    ) -> Result<FilterWatcher<'b, Self::Provider, Log>, Self::Error> {
        let id = U256::zero();
        let bb = self.inner.as_ref().as_ref().unwrap();
        let filter = FilterWatcher::new(id, bb).interval(Duration::from_secs(1));
        Ok(filter)
    }
}

fn log_default() -> Log {
    Log {
        address: H160::zero(),
        topics: Vec::new(),
        data: EthersBytes(Bytes::new()),
        block_hash: None,
        block_number: None,
        transaction_hash: None,
        transaction_index: None,
        log_index: None,
        transaction_log_index: None,
        log_type: None,
        removed: Some(false),
    }
}
