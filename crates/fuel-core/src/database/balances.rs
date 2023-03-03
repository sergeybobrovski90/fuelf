use crate::{
    database::{
        storage::{
            ContractsAssetsMerkleData,
            ContractsAssetsMerkleMetadata,
            SparseMerkleMetadata,
        },
        Column,
        Database,
    },
    state::IterDirection,
};
use fuel_core_storage::{
    not_found,
    tables::ContractsAssets,
    Error as StorageError,
    Mappable,
    MerkleRoot,
    MerkleRootStorage,
    StorageAsMut,
    StorageAsRef,
    StorageInspect,
    StorageMutate,
};
use fuel_core_types::{
    fuel_merkle::sparse::MerkleTree,
    fuel_types::ContractId,
};
use std::borrow::{
    BorrowMut,
    Cow,
};

impl StorageInspect<ContractsAssets> for Database {
    type Error = StorageError;

    fn get(
        &self,
        key: &<ContractsAssets as Mappable>::Key,
    ) -> Result<Option<Cow<<ContractsAssets as Mappable>::OwnedValue>>, Self::Error> {
        self.get(key.as_ref(), Column::ContractsAssets)
            .map_err(Into::into)
    }

    fn contains_key(
        &self,
        key: &<ContractsAssets as Mappable>::Key,
    ) -> Result<bool, Self::Error> {
        self.contains_key(key.as_ref(), Column::ContractsAssets)
            .map_err(Into::into)
    }
}

impl StorageMutate<ContractsAssets> for Database {
    fn insert(
        &mut self,
        key: &<ContractsAssets as Mappable>::Key,
        value: &<ContractsAssets as Mappable>::Value,
    ) -> Result<Option<<ContractsAssets as Mappable>::OwnedValue>, Self::Error> {
        let prev = Database::insert(self, key.as_ref(), Column::ContractsAssets, value)
            .map_err(Into::into);

        // Get latest metadata entry
        let prev_metadata = self
            .iter_all::<Vec<u8>, SparseMerkleMetadata>(
                Column::ContractsAssetsMerkleMetadata,
                Some(IterDirection::Reverse),
            )
            .next()
            .transpose()?
            .map(|(_, metadata)| metadata)
            .unwrap_or_default();

        let root = prev_metadata.root;
        let mut tree: MerkleTree<ContractsAssetsMerkleData, _> = {
            let storage = self.borrow_mut();
            if root == [0; 32].into() {
                // The tree is empty
                MerkleTree::new(storage)
            } else {
                // Load the tree saved in metadata
                MerkleTree::load(storage, &root)
                    .map_err(|err| StorageError::Other(err.into()))?
            }
        };

        // Update the key-value dataset. The key is the contract id and the
        // value is the 32 bytes
        tree.update(&(*key.contract_id()).into(), value.to_be_bytes().as_slice())
            .map_err(|err| StorageError::Other(err.into()))?;

        // Generate new metadata for the updated tree
        let root = tree.root().into();
        let metadata = SparseMerkleMetadata { root };
        self.storage::<ContractsAssetsMerkleMetadata>()
            .insert(key.contract_id(), &metadata)?;

        prev
    }

    fn remove(
        &mut self,
        key: &<ContractsAssets as Mappable>::Key,
    ) -> Result<Option<<ContractsAssets as Mappable>::OwnedValue>, Self::Error> {
        let prev = Database::remove(self, key.as_ref(), Column::ContractsAssets)
            .map_err(Into::into);

        // Get latest metadata entry
        let prev_metadata = self
            .iter_all::<Vec<u8>, SparseMerkleMetadata>(
                Column::ContractsAssetsMerkleMetadata,
                Some(IterDirection::Reverse),
            )
            .next()
            .transpose()?
            .map(|(_, metadata)| metadata)
            .unwrap_or_default();

        let root = prev_metadata.root;
        let mut tree: MerkleTree<ContractsAssetsMerkleData, _> = {
            let storage = self.borrow_mut();
            if root == [0; 32].into() {
                // The tree is empty
                MerkleTree::new(storage)
            } else {
                // Load the tree saved in metadata
                MerkleTree::load(storage, &root)
                    .map_err(|err| StorageError::Other(err.into()))?
            }
        };

        // Update the key-value dataset. The key is the contract id and the
        // value is the 32 bytes
        tree.delete(&(*key.contract_id()).into())
            .map_err(|err| StorageError::Other(err.into()))?;

        // Generate new metadata for the updated tree
        let root = tree.root().into();
        let metadata = SparseMerkleMetadata { root };
        self.storage::<ContractsAssetsMerkleMetadata>()
            .insert(key.contract_id(), &metadata)?;

        prev
    }
}

impl MerkleRootStorage<ContractId, ContractsAssets> for Database {
    fn root(&self, parent: &ContractId) -> Result<MerkleRoot, Self::Error> {
        let metadata = self
            .storage::<ContractsAssetsMerkleMetadata>()
            .get(parent)?
            .ok_or(not_found!("ContractId"))?;
        let root = metadata.root.into();
        Ok(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuel_core_storage::{
        StorageAsMut,
        StorageAsRef,
    };
    use fuel_core_types::fuel_types::{
        AssetId,
        Word,
    };

    #[test]
    fn get() {
        let key = (&ContractId::from([1u8; 32]), &AssetId::new([1u8; 32])).into();
        let balance: Word = 100;

        let database = &mut Database::default();
        database
            .storage::<ContractsAssets>()
            .insert(&key, &balance)
            .unwrap();

        assert_eq!(
            database
                .storage::<ContractsAssets>()
                .get(&key)
                .unwrap()
                .unwrap()
                .into_owned(),
            balance
        );
    }

    #[test]
    fn put() {
        let key = (&ContractId::from([1u8; 32]), &AssetId::new([1u8; 32])).into();
        let balance: Word = 100;

        let database = &mut Database::default();
        database
            .storage::<ContractsAssets>()
            .insert(&key, &balance)
            .unwrap();

        let returned = database
            .storage::<ContractsAssets>()
            .get(&key)
            .unwrap()
            .unwrap();
        assert_eq!(*returned, balance);
    }

    #[test]
    fn remove() {
        let key = (&ContractId::from([1u8; 32]), &AssetId::new([1u8; 32])).into();
        let balance: Word = 100;

        let database = &mut Database::default();
        database
            .storage::<ContractsAssets>()
            .insert(&key, &balance)
            .unwrap();

        database.storage::<ContractsAssets>().remove(&key).unwrap();

        assert!(!database
            .storage::<ContractsAssets>()
            .contains_key(&key)
            .unwrap());
    }

    #[test]
    fn exists() {
        let key = (&ContractId::from([1u8; 32]), &AssetId::new([1u8; 32])).into();
        let balance: Word = 100;

        let database = &mut Database::default();
        database
            .storage::<ContractsAssets>()
            .insert(&key, &balance)
            .unwrap();

        assert!(database
            .storage::<ContractsAssets>()
            .contains_key(&key)
            .unwrap());
    }

    #[test]
    fn root() {
        let key = (&ContractId::from([1u8; 32]), &AssetId::new([1u8; 32])).into();
        let balance: Word = 100;

        let mut database = Database::default();

        StorageMutate::<ContractsAssets>::insert(&mut database, &key, &balance).unwrap();

        let root = database
            .storage::<ContractsAssets>()
            .root(key.contract_id());
        assert!(root.is_ok())
    }
}
