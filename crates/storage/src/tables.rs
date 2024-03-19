//! The module contains definition of storage tables used by default implementation of fuel
//! services.

use crate::Mappable;
use fuel_core_types::{
    blockchain::{
        block::CompressedBlock,
        consensus::Consensus,
        header::{
            ConsensusParametersVersion,
            StateTransitionBytecodeVersion,
        },
    },
    entities::{
        coins::coin::CompressedCoin,
        contract::{
            ContractUtxoInfo,
            ContractsInfoType,
        },
        message::Message,
    },
    fuel_tx::{
        ConsensusParameters,
        Transaction,
        TxId,
        UtxoId,
    },
    fuel_types::{
        BlockHeight,
        ContractId,
        Nonce,
    },
};
pub use fuel_vm_private::storage::{
    ContractsAssets,
    ContractsRawCode,
    ContractsState,
};

/// The table of blocks generated by Fuels validators.
/// Right now, we have only that type of block, but we will support others in the future.
pub struct FuelBlocks;

impl Mappable for FuelBlocks {
    /// Unique identifier of the fuel block.
    type Key = Self::OwnedKey;
    type OwnedKey = BlockHeight;
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

/// Contract info
pub struct ContractsInfo;

impl Mappable for ContractsInfo {
    type Key = Self::OwnedKey;
    type OwnedKey = ContractId;
    type Value = Self::OwnedValue;
    type OwnedValue = ContractsInfoType;
}

/// The table of consensus metadata associated with sealed (finalized) blocks
pub struct SealedBlockConsensus;

impl Mappable for SealedBlockConsensus {
    type Key = Self::OwnedKey;
    type OwnedKey = BlockHeight;
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

/// The storage table of consensus parameters.
pub struct ConsensusParametersVersions;

impl Mappable for ConsensusParametersVersions {
    type Key = Self::OwnedKey;
    type OwnedKey = ConsensusParametersVersion;
    type Value = Self::OwnedValue;
    type OwnedValue = ConsensusParameters;
}

/// The storage table of state transition bytecodes.
pub struct StateTransitionBytecodeVersions;

impl Mappable for StateTransitionBytecodeVersions {
    type Key = Self::OwnedKey;
    type OwnedKey = StateTransitionBytecodeVersion;
    type Value = [u8];
    type OwnedValue = Vec<u8>;
}

/// The module contains definition of merkle-related tables.
pub mod merkle {
    use crate::{
        Mappable,
        MerkleRoot,
    };
    use fuel_core_types::{
        fuel_merkle::{
            binary,
            sparse,
        },
        fuel_tx::ContractId,
        fuel_types::BlockHeight,
    };

    /// The key for the corresponding `DenseMerkleMetadata` type.
    /// The `Latest` variant is used to have the access to the latest dense Merkle tree.
    #[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub enum DenseMetadataKey<PrimaryKey> {
        /// The primary key of the `DenseMerkleMetadata`.
        Primary(PrimaryKey),
        #[default]
        /// The latest `DenseMerkleMetadata` of the table.
        Latest,
    }

    #[cfg(feature = "test-helpers")]
    impl<PrimaryKey> rand::distributions::Distribution<DenseMetadataKey<PrimaryKey>>
        for rand::distributions::Standard
    where
        rand::distributions::Standard: rand::distributions::Distribution<PrimaryKey>,
    {
        fn sample<R: rand::Rng + ?Sized>(
            &self,
            rng: &mut R,
        ) -> DenseMetadataKey<PrimaryKey> {
            DenseMetadataKey::Primary(rng.gen())
        }
    }

    /// Metadata for dense Merkle trees
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
    pub enum DenseMerkleMetadata {
        /// V1 Dense Merkle Metadata
        V1(DenseMerkleMetadataV1),
    }

    impl Default for DenseMerkleMetadata {
        fn default() -> Self {
            Self::V1(Default::default())
        }
    }

    impl DenseMerkleMetadata {
        /// Create a new dense Merkle metadata object from the given Merkle
        /// root and version
        pub fn new(root: MerkleRoot, version: u64) -> Self {
            let metadata = DenseMerkleMetadataV1 { root, version };
            Self::V1(metadata)
        }

        /// Get the Merkle root of the dense Metadata
        pub fn root(&self) -> &MerkleRoot {
            match self {
                DenseMerkleMetadata::V1(metadata) => &metadata.root,
            }
        }

        /// Get the version of the dense Metadata
        pub fn version(&self) -> u64 {
            match self {
                DenseMerkleMetadata::V1(metadata) => metadata.version,
            }
        }
    }

    /// Metadata for dense Merkle trees
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
    pub struct DenseMerkleMetadataV1 {
        /// The root hash of the dense Merkle tree structure
        pub root: MerkleRoot,
        /// The version of the dense Merkle tree structure is equal to the number of
        /// leaves. Every time we append a new leaf to the Merkle tree data set, we
        /// increment the version number.
        pub version: u64,
    }

    impl Default for DenseMerkleMetadataV1 {
        fn default() -> Self {
            let empty_merkle_tree = binary::root_calculator::MerkleRootCalculator::new();
            Self {
                root: empty_merkle_tree.root(),
                version: 0,
            }
        }
    }

    impl From<DenseMerkleMetadataV1> for DenseMerkleMetadata {
        fn from(value: DenseMerkleMetadataV1) -> Self {
            Self::V1(value)
        }
    }

    /// Metadata for sparse Merkle trees
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
    pub enum SparseMerkleMetadata {
        /// V1 Sparse Merkle Metadata
        V1(SparseMerkleMetadataV1),
    }

    impl Default for SparseMerkleMetadata {
        fn default() -> Self {
            Self::V1(Default::default())
        }
    }

    impl SparseMerkleMetadata {
        /// Create a new sparse Merkle metadata object from the given Merkle
        /// root
        pub fn new(root: MerkleRoot) -> Self {
            let metadata = SparseMerkleMetadataV1 { root };
            Self::V1(metadata)
        }

        /// Get the Merkle root stored in the metadata
        pub fn root(&self) -> &MerkleRoot {
            match self {
                SparseMerkleMetadata::V1(metadata) => &metadata.root,
            }
        }
    }

    /// Metadata V1 for sparse Merkle trees
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
    pub struct SparseMerkleMetadataV1 {
        /// The root hash of the sparse Merkle tree structure
        pub root: MerkleRoot,
    }

    impl Default for SparseMerkleMetadataV1 {
        fn default() -> Self {
            let empty_merkle_tree = sparse::in_memory::MerkleTree::new();
            Self {
                root: empty_merkle_tree.root(),
            }
        }
    }

    impl From<SparseMerkleMetadataV1> for SparseMerkleMetadata {
        fn from(value: SparseMerkleMetadataV1) -> Self {
            Self::V1(value)
        }
    }

    /// The table of BMT data for Fuel blocks.
    pub struct FuelBlockMerkleData;

    impl Mappable for FuelBlockMerkleData {
        type Key = u64;
        type OwnedKey = Self::Key;
        type Value = binary::Primitive;
        type OwnedValue = Self::Value;
    }

    /// The metadata table for [`FuelBlockMerkleData`] table.
    pub struct FuelBlockMerkleMetadata;

    impl Mappable for FuelBlockMerkleMetadata {
        type Key = DenseMetadataKey<BlockHeight>;
        type OwnedKey = Self::Key;
        type Value = DenseMerkleMetadata;
        type OwnedValue = Self::Value;
    }

    /// The table of SMT data for Contract assets.
    pub struct ContractsAssetsMerkleData;

    impl Mappable for ContractsAssetsMerkleData {
        type Key = [u8; 32];
        type OwnedKey = Self::Key;
        type Value = sparse::Primitive;
        type OwnedValue = Self::Value;
    }

    /// The metadata table for [`ContractsAssetsMerkleData`] table
    pub struct ContractsAssetsMerkleMetadata;

    impl Mappable for ContractsAssetsMerkleMetadata {
        type Key = ContractId;
        type OwnedKey = Self::Key;
        type Value = SparseMerkleMetadata;
        type OwnedValue = Self::Value;
    }

    /// The table of SMT data for Contract state.
    pub struct ContractsStateMerkleData;

    impl Mappable for ContractsStateMerkleData {
        type Key = [u8; 32];
        type OwnedKey = Self::Key;
        type Value = sparse::Primitive;
        type OwnedValue = Self::Value;
    }

    /// The metadata table for [`ContractsStateMerkleData`] table
    pub struct ContractsStateMerkleMetadata;

    impl Mappable for ContractsStateMerkleMetadata {
        type Key = ContractId;
        type OwnedKey = Self::Key;
        type Value = SparseMerkleMetadata;
        type OwnedValue = Self::Value;
    }
}
