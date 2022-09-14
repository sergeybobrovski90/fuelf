use fuel_core_interfaces::{
    common::{
        fuel_crypto::fuel_types::Bytes32,
        fuel_storage::Mappable,
        fuel_tx::{
            Receipt,
            UtxoId,
        },
    },
    model::FuelBlockDb,
};
use fuel_txpool::types::ContractId;

/// The table of blocks generated by Fuels validators.
/// Right now, we have only that type of block, but we will support others in the future.
pub struct FuelBlocks;

impl Mappable for FuelBlocks {
    /// Unique identifier of the fuel block.
    type Key = Bytes32;
    type SetValue = FuelBlockDb;
    type GetValue = Self::SetValue;
}

/// The latest UTXO id of the contract. The contract's UTXO represents the unique id of the state.
/// After each transaction, old UTXO is consumed, and new UTXO is produced. UTXO is used as an
/// input to the next transaction related to the `ContractId` smart contract.
pub struct ContractsLatestUtxo;

impl Mappable for ContractsLatestUtxo {
    type Key = ContractId;
    /// The latest UTXO id.
    type SetValue = UtxoId;
    type GetValue = Self::SetValue;
}

/// Receipts of different hidden internal operations.
pub struct Receipts;

impl Mappable for Receipts {
    /// Unique identifier of the receipt.
    type Key = Bytes32;
    type SetValue = [Receipt];
    type GetValue = Vec<Receipt>;
}

// TODO: Add macro to define all common tables to avoid copy/paste of the code.
// TODO: Add macro to define common unit tests.
