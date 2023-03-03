use crate::{
    database::{
        storage::{
            ContractsStateMerkleData,
            ContractsStateMerkleMetadata,
            SparseMerkleMetadata,
        },
        Column,
        Database,
    },
    state::IterDirection,
};
use fuel_core_storage::{
    tables::ContractsState,
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
    fuel_merkle::sparse::{
        in_memory,
        MerkleTree,
    },
    fuel_types::ContractId,
};
use std::borrow::{
    BorrowMut,
    Cow,
};

impl StorageInspect<ContractsState> for Database {
    type Error = StorageError;

    fn get(
        &self,
        key: &<ContractsState as Mappable>::Key,
    ) -> Result<Option<Cow<<ContractsState as Mappable>::OwnedValue>>, Self::Error> {
        self.get(key.as_ref(), Column::ContractsState)
            .map_err(Into::into)
    }

    fn contains_key(
        &self,
        key: &<ContractsState as Mappable>::Key,
    ) -> Result<bool, Self::Error> {
        self.contains_key(key.as_ref(), Column::ContractsState)
            .map_err(Into::into)
    }
}

impl StorageMutate<ContractsState> for Database {
    fn insert(
        &mut self,
        key: &<ContractsState as Mappable>::Key,
        value: &<ContractsState as Mappable>::Value,
    ) -> Result<Option<<ContractsState as Mappable>::OwnedValue>, Self::Error> {
        let prev = Database::insert(self, key.as_ref(), Column::ContractsState, value)
            .map_err(Into::into);

        // Get latest metadata entry
        let prev_metadata = self
            .iter_all::<Vec<u8>, SparseMerkleMetadata>(
                Column::ContractsStateMerkleMetadata,
                Some(IterDirection::Reverse),
            )
            .next()
            .transpose()?
            .map(|(_, metadata)| metadata)
            .unwrap_or_default();

        let root = prev_metadata.root;
        let mut tree: MerkleTree<ContractsStateMerkleData, _> = {
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
        tree.update(&(*key.contract_id()).into(), value.as_slice())
            .map_err(|err| StorageError::Other(err.into()))?;

        // Generate new metadata for the updated tree
        let root = tree.root().into();
        let metadata = SparseMerkleMetadata { root };
        self.storage::<ContractsStateMerkleMetadata>()
            .insert(key.contract_id(), &metadata)?;

        prev
    }

    fn remove(
        &mut self,
        key: &<ContractsState as Mappable>::Key,
    ) -> Result<Option<<ContractsState as Mappable>::OwnedValue>, Self::Error> {
        let prev = Database::remove(self, key.as_ref(), Column::ContractsState)
            .map_err(Into::into);

        // Get latest metadata entry
        let prev_metadata = self
            .iter_all::<Vec<u8>, SparseMerkleMetadata>(
                Column::ContractsStateMerkleMetadata,
                Some(IterDirection::Reverse),
            )
            .next()
            .transpose()?
            .map(|(_, metadata)| metadata)
            .unwrap_or_default();

        let root = prev_metadata.root;
        let mut tree: MerkleTree<ContractsStateMerkleData, _> = {
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
        self.storage::<ContractsStateMerkleMetadata>()
            .insert(key.contract_id(), &metadata)?;

        prev
    }
}

impl MerkleRootStorage<ContractId, ContractsState> for Database {
    fn root(&self, parent: &ContractId) -> Result<MerkleRoot, Self::Error> {
        let metadata = self.storage::<ContractsStateMerkleMetadata>().get(parent)?;
        let root = metadata
            .map(|metadata| metadata.root.into())
            .unwrap_or_else(|| in_memory::MerkleTree::new().root());
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
    use fuel_core_types::fuel_types::Bytes32;

    #[test]
    fn get() {
        let key = (&ContractId::from([1u8; 32]), &Bytes32::from([1u8; 32])).into();
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let database = &mut Database::default();
        database
            .storage::<ContractsState>()
            .insert(&key, &stored_value)
            .unwrap();

        assert_eq!(
            *database
                .storage::<ContractsState>()
                .get(&key)
                .unwrap()
                .unwrap(),
            stored_value
        );
    }

    #[test]
    fn put() {
        let key = (&ContractId::from([1u8; 32]), &Bytes32::from([1u8; 32])).into();
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let database = &mut Database::default();
        database
            .storage::<ContractsState>()
            .insert(&key, &stored_value)
            .unwrap();

        let returned: Bytes32 = *database
            .storage::<ContractsState>()
            .get(&key)
            .unwrap()
            .unwrap();
        assert_eq!(returned, stored_value);
    }

    #[test]
    fn remove() {
        let key = (&ContractId::from([1u8; 32]), &Bytes32::from([1u8; 32])).into();
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let database = &mut Database::default();
        database
            .storage::<ContractsState>()
            .insert(&key, &stored_value)
            .unwrap();

        database.storage::<ContractsState>().remove(&key).unwrap();

        assert!(!database
            .storage::<ContractsState>()
            .contains_key(&key)
            .unwrap());
    }

    #[test]
    fn exists() {
        let key = (&ContractId::from([1u8; 32]), &Bytes32::from([1u8; 32])).into();
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let database = &mut Database::default();
        database
            .storage::<ContractsState>()
            .insert(&key, &stored_value)
            .unwrap();

        assert!(database
            .storage::<ContractsState>()
            .contains_key(&key)
            .unwrap());
    }

    #[test]
    fn root() {
        let key = (&ContractId::from([1u8; 32]), &Bytes32::from([1u8; 32])).into();
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let mut database = Database::default();

        StorageMutate::<ContractsState>::insert(&mut database, &key, &stored_value)
            .unwrap();

        let root = database.storage::<ContractsState>().root(key.contract_id());
        assert!(root.is_ok())
    }
}
