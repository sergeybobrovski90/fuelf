//! The module contains definition of storage tables used by default implementation of fuel
//! services.

use crate::Mappable;
use fuel_core_types::{
    blockchain::{
        block::CompressedBlock,
        consensus::Consensus,
        primitives::BlockId,
    },
    entities::{
        coins::coin::CompressedCoin,
        contract::ContractUtxoInfo,
        message::Message,
    },
    fuel_tx::{
        Receipt,
        Transaction,
        TxId,
        UtxoId,
    },
    fuel_types::{
        Bytes32,
        ContractId,
        Nonce,
    },
};
pub use fuel_vm_private::storage::{
    ContractsAssets,
    ContractsInfo,
    ContractsRawCode,
    ContractsState,
};

/// The table of blocks generated by Fuels validators.
/// Right now, we have only that type of block, but we will support others in the future.
pub struct FuelBlocks;

impl Mappable for FuelBlocks {
    /// Unique identifier of the fuel block.
    type Key = Self::OwnedKey;
    // TODO: Seems it would be faster to use `BlockHeight` as primary key.
    type OwnedKey = BlockId;
    type Value = Self::OwnedValue;
    type OwnedValue = CompressedBlock;
}

/// The latest UTXO info of the contract. The contract's UTXO represents the unique id of the state.
/// After each transaction, old UTXO is consumed, and new UTXO is produced. UTXO is used as an
/// input to the next transaction related to the `ContractId` smart contract.
pub struct ContractsLatestUtxo;

impl Mappable for ContractsLatestUtxo {
    type Key = Self::OwnedKey;
    type OwnedKey = ContractId;
    /// The latest UTXO info
    type Value = Self::OwnedValue;
    type OwnedValue = ContractUtxoInfo;
}

/// Receipts of different hidden internal operations.
pub struct Receipts;

impl Mappable for Receipts {
    /// Unique identifier of the transaction.
    type Key = Self::OwnedKey;
    type OwnedKey = Bytes32;
    type Value = [Receipt];
    type OwnedValue = Vec<Receipt>;
}

/// The table of consensus metadata associated with sealed (finalized) blocks
pub struct SealedBlockConsensus;

impl Mappable for SealedBlockConsensus {
    type Key = Self::OwnedKey;
    type OwnedKey = BlockId;
    type Value = Self::OwnedValue;
    type OwnedValue = Consensus;
}

/// The storage table of coins. Each [`CompressedCoin`]
/// is represented by unique `UtxoId`.
pub struct Coins;

impl Mappable for Coins {
    type Key = Self::OwnedKey;
    type OwnedKey = UtxoId;
    type Value = Self::OwnedValue;
    type OwnedValue = CompressedCoin;
}

/// The storage table of bridged Ethereum message.
pub struct Messages;

impl Mappable for Messages {
    type Key = Self::OwnedKey;
    type OwnedKey = Nonce;
    type Value = Self::OwnedValue;
    type OwnedValue = Message;
}

/// The storage table that indicates if the message is spent or not.
pub struct SpentMessages;

impl Mappable for SpentMessages {
    type Key = Self::OwnedKey;
    type OwnedKey = Nonce;
    type Value = Self::OwnedValue;
    type OwnedValue = ();
}

/// The storage table of confirmed transactions.
pub struct Transactions;

impl Mappable for Transactions {
    type Key = Self::OwnedKey;
    type OwnedKey = TxId;
    type Value = Self::OwnedValue;
    type OwnedValue = Transaction;
}

/// The storage table of processed transactions that were executed in the past.
/// The table helps to drop duplicated transactions.
pub struct ProcessedTransactions;

impl Mappable for ProcessedTransactions {
    type Key = Self::OwnedKey;
    type OwnedKey = TxId;
    type Value = Self::OwnedValue;
    type OwnedValue = ();
}

// TODO: Add macro to define all common tables to avoid copy/paste of the code.
// TODO: Add macro to define common unit tests.
