//! Relayed (forced) transaction entity types

use crate::{
    blockchain::primitives::DaBlockHeight,
    fuel_crypto,
    fuel_types::Bytes32,
};

/// Transaction sent from the DA layer to fuel by the relayer
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RelayedTransaction {
    /// V1 version of the relayed transaction
    V1(RelayedTransactionV1),
}

/// The V1 version of the relayed transaction
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayedTransactionV1 {
    /// The max gas that this transaction can consume
    pub max_gas: u64,
    /// The serialized transaction transmitted from the bridge
    pub serialized_transaction: Vec<u8>,
    /// The block height from the parent da layer that originated this transaction
    pub da_height: DaBlockHeight,
}

/// The hash of a relayed transaction
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    derive_more::Display,
    derive_more::From,
)]
pub struct RelayedTransactionId(Bytes32);

impl RelayedTransaction {
    /// Get the DA height the transaction originated from
    pub fn da_height(&self) -> DaBlockHeight {
        match self {
            RelayedTransaction::V1(transaction) => transaction.da_height,
        }
    }

    /// The hash of the relayed transaction
    pub fn relayed_id(&self) -> RelayedTransactionId {
        match &self {
            RelayedTransaction::V1(tx) => tx.relayed_transaction_id(),
        }
    }
}

impl RelayedTransactionV1 {
    /// The hash of the relayed transaction (max_gas (big endian) || serialized_transaction)
    pub fn relayed_transaction_id(&self) -> RelayedTransactionId {
        let hasher = fuel_crypto::Hasher::default()
            .chain(self.max_gas.to_be_bytes())
            .chain(self.serialized_transaction.as_slice());
        // TODO: We need some kind of assurance from L1 that this ID is unique
        RelayedTransactionId((*hasher.finalize()).into())
    }
}

impl From<RelayedTransactionV1> for RelayedTransaction {
    fn from(relayed_transaction: RelayedTransactionV1) -> Self {
        RelayedTransaction::V1(relayed_transaction)
    }
}
