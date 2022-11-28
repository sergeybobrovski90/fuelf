pub use crate::common::fuel_vm::storage::{
    ContractsAssets,
    ContractsInfo,
    ContractsRawCode,
    ContractsState,
};
use crate::{
    common::{
        fuel_storage::Mappable,
        fuel_tx::{
            Transaction,
            UtxoId,
        },
        fuel_types::{
            Bytes32,
            MessageId,
        },
        fuel_vm::prelude::InterpreterError,
    },
    model::{
        Coin,
        Message,
    },
};
use std::io::ErrorKind;
use thiserror::Error;

/// The storage table of coins. Each [`Coin`](crate::model::Coin) is represented by unique `UtxoId`.
pub struct Coins;

impl Mappable for Coins {
    type Key = UtxoId;
    type SetValue = Coin;
    type GetValue = Self::SetValue;
}

/// The storage table of bridged Ethereum [`Message`](crate::model::Message)s.
pub struct Messages;

impl Mappable for Messages {
    type Key = MessageId;
    type SetValue = Message;
    type GetValue = Self::SetValue;
}

/// The storage table of confirmed transactions.
pub struct Transactions;

impl Mappable for Transactions {
    type Key = Bytes32;
    type SetValue = Transaction;
    type GetValue = Self::SetValue;
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("error performing binary serialization")]
    Codec,
    #[error("Failed to initialize chain")]
    ChainAlreadyInitialized,
    #[error("Chain is not yet initialized")]
    ChainUninitialized,
    #[error("Invalid database version")]
    InvalidDatabaseVersion,
    #[error("error occurred in the underlying datastore `{0}`")]
    DatabaseError(Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<Error> for std::io::Error {
    fn from(e: Error) -> Self {
        std::io::Error::new(ErrorKind::Other, e)
    }
}

#[derive(Debug, Error)]
pub enum KvStoreError {
    #[error("generic error occurred")]
    Error(Box<dyn std::error::Error + Send + Sync>),
    /// This error should be created with `not_found` macro.
    #[error("resource of type `{0}` was not found at the: {1}")]
    NotFound(&'static str, &'static str),
}

/// Creates `KvStoreError::NotFound` error with file and line information inside.
///
/// # Examples
///
/// ```
/// use fuel_core_interfaces::not_found;
/// use fuel_core_interfaces::db::Messages;
///
/// let string_type = not_found!("BlockId");
/// let mappable_type = not_found!(Messages);
/// let mappable_path = not_found!(fuel_core_interfaces::db::Coins);
/// ```
#[macro_export]
macro_rules! not_found {
    ($name: literal) => {
        $crate::db::KvStoreError::NotFound($name, concat!(file!(), ":", line!()))
    };
    ($ty: path) => {
        $crate::db::KvStoreError::NotFound(
            ::core::any::type_name::<
                <$ty as $crate::common::fuel_storage::Mappable>::GetValue,
            >(),
            concat!(file!(), ":", line!()),
        )
    };
}

impl From<Error> for KvStoreError {
    fn from(e: Error) -> Self {
        KvStoreError::Error(Box::new(e))
    }
}

impl From<KvStoreError> for Error {
    fn from(e: KvStoreError) -> Self {
        Error::DatabaseError(Box::new(e))
    }
}

impl From<KvStoreError> for std::io::Error {
    fn from(e: KvStoreError) -> Self {
        std::io::Error::new(ErrorKind::Other, e)
    }
}

impl From<Error> for InterpreterError {
    fn from(e: Error) -> Self {
        InterpreterError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

impl From<KvStoreError> for InterpreterError {
    fn from(e: KvStoreError) -> Self {
        InterpreterError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn not_found_output() {
        #[rustfmt::skip]
        assert_eq!(
            format!("{}", not_found!("BlockId")),
            format!("resource of type `BlockId` was not found at the: {}:{}", file!(), line!() - 1)
        );
        #[rustfmt::skip]
        assert_eq!(
            format!("{}", not_found!(Coins)),
            format!("resource of type `fuel_core_interfaces::model::coin::Coin` was not found at the: {}:{}", file!(), line!() - 1)
        );
    }
}
