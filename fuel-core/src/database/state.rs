use crate::database::columns::CONTRACTS_STATE;
use crate::database::Database;
use crate::state::in_memory::memory_store::MemoryStore;
use crate::state::in_memory::transaction::MemoryTransactionView;
use crate::state::{ColumnId, DataSource, Error, MultiKey};
use fuel_vm::crypto;
use fuel_vm::data::{DataError, InterpreterStorage, MerkleStorage};
use fuel_vm::prelude::{Address, Bytes32, Color, Contract, ContractId, Salt, Word};
use itertools::Itertools;
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
use std::sync::Arc;

impl MerkleStorage<ContractId, Bytes32, Bytes32> for Database {
    fn insert(
        &mut self,
        parent: &ContractId,
        key: &Bytes32,
        value: &Bytes32,
    ) -> Result<Option<Bytes32>, DataError> {
        let key = MultiKey::new((parent, key));
        Database::insert(self, key.as_ref().to_vec(), CONTRACTS_STATE, value.clone())
            .map_err(Into::into)
    }

    fn remove(&mut self, parent: &ContractId, key: &Bytes32) -> Result<Option<Bytes32>, DataError> {
        let key = MultiKey::new((parent, key));
        Database::remove(self, key.as_ref(), CONTRACTS_STATE).map_err(Into::into)
    }

    fn get(&self, parent: &ContractId, key: &Bytes32) -> Result<Option<Bytes32>, DataError> {
        let key = MultiKey::new((parent, key));
        self.get(key.as_ref(), CONTRACTS_STATE).map_err(Into::into)
    }

    fn contains_key(&self, parent: &ContractId, key: &Bytes32) -> Result<bool, DataError> {
        let key = MultiKey::new((parent, key));
        self.exists(key.as_ref(), CONTRACTS_STATE)
            .map_err(Into::into)
    }

    fn root(&mut self, parent: &ContractId) -> Result<Bytes32, DataError> {
        let items: Vec<_> =
            Database::iter_all::<Vec<u8>, Bytes32>(self, CONTRACTS_STATE).try_collect()?;

        let root = items
            .iter()
            .filter_map(|(key, value)| {
                (&key[..parent.len()] == parent.as_ref()).then(|| (key, value))
            })
            .sorted_by_key(|t| t.0)
            .map(|(_, value)| value);

        Ok(crypto::ephemeral_merkle_root(root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get() {
        let storage_id: (ContractId, Bytes32) =
            (ContractId::from([1u8; 32]), Bytes32::from([1u8; 32]));
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let database = Database::default();
        database
            .insert(
                MultiKey::new(storage_id),
                CONTRACTS_STATE,
                stored_value.clone(),
            )
            .unwrap();

        assert_eq!(
            MerkleStorage::<ContractId, Bytes32, Bytes32>::get(
                &database,
                &storage_id.0,
                &storage_id.1
            )
            .unwrap()
            .unwrap(),
            stored_value
        );
    }

    #[test]
    fn put() {
        let storage_id: (ContractId, Bytes32) =
            (ContractId::from([1u8; 32]), Bytes32::from([1u8; 32]));
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let mut database = Database::default();
        MerkleStorage::<ContractId, Bytes32, Bytes32>::insert(
            &mut database,
            &storage_id.0,
            &storage_id.1,
            &stored_value,
        )
        .unwrap();

        let returned: Bytes32 = database
            .get(MultiKey::new(storage_id).as_ref(), CONTRACTS_STATE)
            .unwrap()
            .unwrap();
        assert_eq!(returned, stored_value);
    }

    #[test]
    fn remove() {
        let storage_id: (ContractId, Bytes32) =
            (ContractId::from([1u8; 32]), Bytes32::from([1u8; 32]));
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let mut database = Database::default();
        database
            .insert(
                MultiKey::new(storage_id),
                CONTRACTS_STATE,
                stored_value.clone(),
            )
            .unwrap();

        MerkleStorage::<ContractId, Bytes32, Bytes32>::remove(
            &mut database,
            &storage_id.0,
            &storage_id.1,
        )
        .unwrap();

        assert!(!database
            .exists(MultiKey::new(storage_id).as_ref(), CONTRACTS_STATE)
            .unwrap());
    }

    #[test]
    fn exists() {
        let storage_id: (ContractId, Bytes32) =
            (ContractId::from([1u8; 32]), Bytes32::from([1u8; 32]));
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let database = Database::default();
        database
            .insert(
                MultiKey::new(storage_id.clone()),
                CONTRACTS_STATE,
                stored_value.clone(),
            )
            .unwrap();

        assert!(MerkleStorage::<ContractId, Bytes32, Bytes32>::contains_key(
            &database,
            &storage_id.0,
            &storage_id.1
        )
        .unwrap());
    }

    #[test]
    fn root() {
        let storage_id: (ContractId, Bytes32) =
            (ContractId::from([1u8; 32]), Bytes32::from([1u8; 32]));
        let stored_value: Bytes32 = Bytes32::from([2u8; 32]);

        let mut database = Database::default();

        MerkleStorage::<ContractId, Bytes32, Bytes32>::insert(
            &mut database,
            &storage_id.0,
            &storage_id.1,
            &stored_value,
        )
        .unwrap();

        let root =
            MerkleStorage::<ContractId, Bytes32, Bytes32>::root(&mut database, &storage_id.0);
        assert!(root.is_ok())
    }
}
