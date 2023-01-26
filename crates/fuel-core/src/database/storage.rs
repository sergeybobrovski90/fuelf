use crate::database::{
    Column,
    Database,
};
use fuel_core_storage::{
    Error as StorageError,
    Mappable,
    Result as StorageResult,
    StorageInspect,
    StorageMutate,
};
use fuel_core_types::{
    blockchain::primitives::{
        BlockHeight,
        BlockId,
    },
    fuel_merkle::binary,
    fuel_types::Bytes32,
};
use serde::{
    de::DeserializeOwned,
    Serialize,
};
use std::borrow::Cow;

/// Metadata for dense Merkle trees
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct DenseMerkleMetadata {
    /// The root hash of the dense Merkle tree structure
    pub root: Bytes32,
    /// The version of the dense Merkle tree structure is equal to the number of
    /// leaves. Every time we append a new leaf to the Merkle tree data set, we
    /// increment the version number.
    pub version: u64,
}

impl Default for DenseMerkleMetadata {
    fn default() -> Self {
        let mut empty_merkle_tree = binary::in_memory::MerkleTree::new();
        Self {
            root: empty_merkle_tree.root().into(),
            version: 0,
        }
    }
}

/// The table of fuel block's secondary key - `BlockHeight`.
/// It links the `BlockHeight` to corresponding `BlockId`.
pub struct FuelBlockSecondaryKeyBlockHeights;

impl Mappable for FuelBlockSecondaryKeyBlockHeights {
    /// Secondary key - `BlockHeight`.
    type Key = BlockHeight;
    type OwnedKey = Self::Key;
    /// Primary key - `BlockId`.
    type Value = BlockId;
    type OwnedValue = Self::Value;
}

/// The table of BMT MMR data for the fuel blocks.
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
    type Key = BlockHeight;
    type OwnedKey = Self::Key;
    type Value = DenseMerkleMetadata;
    type OwnedValue = Self::Value;
}

/// The table has a corresponding column in the database.
///
/// Using this trait allows the configured mappable type to have its'
/// database integration auto-implemented for single column interactions.
///
/// If the mappable type requires access to multiple columns then its'
/// storage interfaces should be manually implemented.
trait DatabaseColumn {
    /// The column of the table.
    fn column() -> Column;
}

impl DatabaseColumn for FuelBlockSecondaryKeyBlockHeights {
    fn column() -> Column {
        Column::FuelBlockSecondaryKeyBlockHeights
    }
}

impl DatabaseColumn for FuelBlockMerkleData {
    fn column() -> Column {
        Column::FuelBlockMerkleData
    }
}

impl DatabaseColumn for FuelBlockMerkleMetadata {
    fn column() -> Column {
        Column::FuelBlockMerkleMetadata
    }
}

impl<T> StorageInspect<T> for Database
where
    T: Mappable + DatabaseColumn,
    T::Key: ToDatabaseKey,
    T::OwnedValue: DeserializeOwned,
{
    type Error = StorageError;

    fn get(&self, key: &T::Key) -> StorageResult<Option<Cow<T::OwnedValue>>> {
        self.get(key.database_key().as_ref(), T::column())
            .map_err(Into::into)
    }

    fn contains_key(&self, key: &T::Key) -> StorageResult<bool> {
        self.exists(key.database_key().as_ref(), T::column())
            .map_err(Into::into)
    }
}

impl<T> StorageMutate<T> for Database
where
    T: Mappable + DatabaseColumn,
    T::Key: ToDatabaseKey,
    T::Value: Serialize,
    T::OwnedValue: DeserializeOwned,
{
    fn insert(
        &mut self,
        key: &T::Key,
        value: &T::Value,
    ) -> StorageResult<Option<T::OwnedValue>> {
        Database::insert(self, key.database_key().as_ref(), T::column(), value)
            .map_err(Into::into)
    }

    fn remove(&mut self, key: &T::Key) -> StorageResult<Option<T::OwnedValue>> {
        Database::remove(self, key.database_key().as_ref(), T::column())
            .map_err(Into::into)
    }
}

// TODO: Implement this trait for all keys.
//  -> After replace all common implementation with blanket, if possible.
/// Some keys requires pre-processing that could change their type.
pub trait ToDatabaseKey {
    /// A new type of prepared database key that can be converted into bytes.
    type Type: AsRef<[u8]>;

    /// Coverts the key into database key that supports byte presentation.
    fn database_key(&self) -> Self::Type;
}

impl ToDatabaseKey for BlockHeight {
    type Type = [u8; 4];

    fn database_key(&self) -> Self::Type {
        self.to_bytes()
    }
}

impl ToDatabaseKey for u64 {
    type Type = [u8; 8];

    fn database_key(&self) -> Self::Type {
        self.to_be_bytes()
    }
}
