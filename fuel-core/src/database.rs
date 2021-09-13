#[cfg(feature = "default")]
use crate::database::columns::COLUMN_NUM;
use crate::database::transactional::DatabaseTransaction;
use crate::state::in_memory::memory_store::MemoryStore;
#[cfg(feature = "default")]
use crate::state::rocks_db::RocksDb;
use crate::state::{ColumnId, DataSource, Error};
use fuel_vm::data::{DataError, InterpreterStorage};
use fuel_vm::prelude::{Address, Bytes32};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
#[cfg(feature = "default")]
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

pub mod balances;
pub mod block;
pub mod code_root;
pub mod coin;
pub mod contracts;
mod receipts;
pub mod state;
pub mod transaction;
pub mod transactional;

// Crude way to invalidate incompatible databases,
// can be used to perform migrations in the future.
pub const VERSION: u32 = 0;

pub(crate) mod columns {
    pub const DB_VERSION_COLUMN: u32 = 0;
    pub const CONTRACTS: u32 = 1;
    pub const CONTRACTS_CODE_ROOT: u32 = 2;
    pub const CONTRACTS_STATE: u32 = 3;
    pub const BALANCES: u32 = 4;
    pub const COIN: u32 = 5;
    pub const TRANSACTIONS: u32 = 6;
    pub const RECEIPTS: u32 = 7;
    pub const BLOCKS: u32 = 8;

    // Number of columns
    #[cfg(feature = "default")]
    pub const COLUMN_NUM: u32 = 6;
}

pub trait DatabaseTrait: InterpreterStorage + AsRef<Database> + Debug + Send + Sync {
    fn transaction(&self) -> DatabaseTransaction;
}

#[derive(Clone, Debug)]
pub struct SharedDatabase(pub Arc<dyn DatabaseTrait>);

impl Default for SharedDatabase {
    fn default() -> Self {
        SharedDatabase(Arc::new(Database::default()))
    }
}

#[derive(Clone, Debug)]
pub struct Database {
    data: DataSource,
}

impl Database {
    #[cfg(feature = "default")]
    pub fn open(path: &Path) -> Result<Self, Error> {
        let db = RocksDb::open(path, COLUMN_NUM)?;

        Ok(Database { data: Arc::new(db) })
    }

    fn insert<K: Into<Vec<u8>>, V: Serialize + DeserializeOwned>(
        &self,
        key: K,
        column: ColumnId,
        value: V,
    ) -> Result<Option<V>, Error> {
        let result = self.data.put(
            key.into(),
            column,
            bincode::serialize(&value).map_err(|_| Error::Codec)?,
        )?;
        if let Some(previous) = result {
            Ok(Some(
                bincode::deserialize(&previous).map_err(|_| Error::Codec)?,
            ))
        } else {
            Ok(None)
        }
    }

    fn remove<V: DeserializeOwned>(
        &self,
        key: &[u8],
        column: ColumnId,
    ) -> Result<Option<V>, Error> {
        self.data
            .delete(key, column)?
            .map(|val| bincode::deserialize(&val).map_err(|_| Error::Codec))
            .transpose()
    }

    fn get<V: DeserializeOwned>(&self, key: &[u8], column: ColumnId) -> Result<Option<V>, Error> {
        self.data
            .get(key, column)?
            .map(|val| bincode::deserialize(&val).map_err(|_| Error::Codec))
            .transpose()
    }

    fn exists(&self, key: &[u8], column: ColumnId) -> Result<bool, Error> {
        self.data.exists(key, column)
    }

    fn iter_all<K, V>(&self, column: ColumnId) -> impl Iterator<Item = Result<(K, V), Error>> + '_
    where
        K: From<Vec<u8>>,
        V: DeserializeOwned,
    {
        self.data.iter_all(column).map(|(key, value)| {
            let key = K::from(key);
            let value: V = bincode::deserialize(&value).map_err(|_| Error::Codec)?;
            Ok((key, value))
        })
    }
}

impl AsRef<Database> for Database {
    fn as_ref(&self) -> &Database {
        &self
    }
}

impl DatabaseTrait for Database {
    fn transaction(&self) -> DatabaseTransaction {
        self.into()
    }
}

/// Construct an in-memory database
impl Default for Database {
    fn default() -> Self {
        Self {
            data: Arc::new(MemoryStore::default()),
        }
    }
}

impl InterpreterStorage for Database {
    fn block_height(&self) -> Result<u32, DataError> {
        Ok(Default::default())
    }

    fn block_hash(&self, _block_height: u32) -> Result<Bytes32, DataError> {
        Ok(Default::default())
    }

    fn coinbase(&self) -> Result<Address, DataError> {
        Ok(Default::default())
    }
}

pub trait KvStore<K, V> {
    fn insert(&self, key: &K, value: &V) -> Result<Option<V>, KvStoreError>;
    fn remove(&self, key: &K) -> Result<Option<V>, KvStoreError>;
    fn get(&self, key: &K) -> Result<Option<V>, KvStoreError>;
    fn contains_key(&self, key: &K) -> Result<bool, KvStoreError>;
}

#[derive(Debug, Error)]
pub enum KvStoreError {
    #[error("generic error occurred")]
    Error(Box<dyn std::error::Error + Send>),
    #[error("resource not found")]
    NotFound,
}

impl From<bincode::Error> for KvStoreError {
    fn from(e: bincode::Error) -> Self {
        KvStoreError::Error(Box::new(e))
    }
}

impl From<crate::state::Error> for KvStoreError {
    fn from(e: Error) -> Self {
        KvStoreError::Error(Box::new(e))
    }
}

impl From<crate::state::Error> for DataError {
    fn from(e: Error) -> Self {
        panic!("No valid DataError variants to construct {:?}", e)
    }
}
