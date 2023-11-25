use fuel_core_storage::test_helpers::MockStorageMethods;
use fuel_core_storage::transactional::StorageTransaction;
use fuel_core_storage::{Mappable, MerkleRoot, Result as StorageResult, StorageInspect, StorageMutate, StorageRead, StorageSize};
use fuel_core_storage::database::{DatabaseColumnIterator, FuelBlockTrait, FuelStateTrait, TxIdOwnerRecorder, VmDatabaseTrait};
use fuel_core_storage::Error as StorageError;
use fuel_core_types::blockchain::header::ConsensusHeader;
use fuel_core_types::fuel_tx::ContractId;
use fuel_core_types::fuel_types::{Address, BlockHeight, Bytes32, Nonce};
use fuel_core_types::services::txpool::TransactionStatus;
use crate::vm_database::VmDatabase;
use fuel_core_storage::MerkleRootStorage;
use fuel_core_storage::database::MessageIsSpent;
use fuel_core_storage::iter::IterDirection;
use fuel_core_storage::transactional::Transactional;
use fuel_core_storage::transactional::Transaction;
use fuel_core_types::blockchain::primitives::BlockId;
use fuel_core_types::tai64::Tai64;


mockall::mock! {
    /// The mocked storage is useful to test functionality build on top of the `StorageInspect`,
    /// `StorageMutate`, and `MerkleRootStorage` traits.
    pub Storage {}

    impl MockStorageMethods for Storage {
        fn get<M: Mappable + 'static>(
            &self,
            key: &M::Key,
        ) -> StorageResult<Option<std::borrow::Cow<'static, M::OwnedValue>>>;

        fn contains_key<M: Mappable + 'static>(&self, key: &M::Key) -> StorageResult<bool>;

        fn insert<M: Mappable + 'static>(
            &mut self,
            key: &M::Key,
            value: &M::Value,
        ) -> StorageResult<Option<M::OwnedValue>>;

        fn remove<M: Mappable + 'static>(
            &mut self,
            key: &M::Key,
        ) -> StorageResult<Option<M::OwnedValue>>;

        fn root<Key: 'static, M: Mappable + 'static>(&self, key: &Key) -> StorageResult<MerkleRoot>;

        fn size_of_value<M: Mappable + 'static>(&self, key: &M::Key) -> StorageResult<Option<usize>>;
    }

    impl Transactional for Storage {
        type Storage = Self;

        fn transaction(&self) -> StorageTransaction<Self>;
    }

    impl Transaction<Self> for Storage {
        fn commit(&mut self) -> StorageResult<()>;
    }

    impl Clone for Storage {
        fn clone(&self) -> Self;
    }
}


impl MockStorage {
    /// Packs `self` into one more `MockStorage` and implements `Transactional` trait by this move.
    pub fn into_transactional(self) -> MockStorage {
        let mut db = MockStorage::default();
        db.expect_transaction()
            .return_once(move || StorageTransaction::new(self));
        db
    }
}

impl AsRef<MockStorage> for MockStorage {
    fn as_ref(&self) -> &MockStorage {
        self
    }
}

impl AsMut<MockStorage> for MockStorage {
    fn as_mut(&mut self) -> &mut MockStorage {
        self
    }
}

impl<M> StorageSize<M> for MockStorage
    where
        M: Mappable + 'static
{
    fn size_of_value(&self, key: &M::Key) -> Result<Option<usize>, Self::Error> {
        MockStorageMethods::size_of_value::<M>(self, key)
    }
}

impl<M> StorageRead<M> for MockStorage
    where
        M: Mappable + 'static
{
    fn read(&self, key: &M::Key, buf: &mut [u8]) -> Result<Option<usize>, Self::Error> {
        todo!()
    }

    fn read_alloc(&self, key: &M::Key) -> Result<Option<Vec<u8>>, Self::Error> {
        todo!()
    }
}

impl<M> StorageInspect<M> for MockStorage
where
    M: Mappable + 'static,
{
    type Error = StorageError;

    fn get(
        &self,
        key: &M::Key,
    ) -> StorageResult<Option<std::borrow::Cow<M::OwnedValue>>> {
        MockStorageMethods::get::<M>(self, key)
    }

    fn contains_key(&self, key: &M::Key) -> StorageResult<bool> {
        MockStorageMethods::contains_key::<M>(self, key)
    }
}

impl<M> StorageMutate<M> for MockStorage
where
    M: Mappable + 'static,
{
    fn insert(
        &mut self,
        key: &M::Key,
        value: &M::Value,
    ) -> StorageResult<Option<M::OwnedValue>> {
        MockStorageMethods::insert::<M>(self, key, value)
    }

    fn remove(&mut self, key: &M::Key) -> StorageResult<Option<M::OwnedValue>> {
        MockStorageMethods::remove::<M>(self, key)
    }
}

impl<Key, M> MerkleRootStorage<Key, M> for MockStorage
where
    Key: 'static,
    M: Mappable + 'static,
{
    fn root(&self, key: &Key) -> StorageResult<MerkleRoot> {
        MockStorageMethods::root::<Key, M>(self, key)
    }
}

impl MessageIsSpent for MockStorage {
    type Error = StorageError;

    fn message_is_spent(&self, _nonce: &Nonce) -> Result<bool, StorageError> {
        todo!()
    }
}

impl TxIdOwnerRecorder for MockStorage {
    type Error = StorageError;

    fn record_tx_id_owner(
        &self,
        _owner: &Address,
        _block_height: BlockHeight,
        _tx_idx: u16,
        _tx_id: &Bytes32,
    ) -> Result<Option<Bytes32>, Self::Error> {
        todo!()
    }

    fn update_tx_status(
        &self,
        _id: &Bytes32,
        _status: TransactionStatus,
    ) -> Result<Option<TransactionStatus>, Self::Error> {
        todo!()
    }
}

impl VmDatabaseTrait for MockStorage {
    type Data = VmDatabase<MockStorage>;

    fn new<T>(&self, _header: &ConsensusHeader<T>, _coinbase: ContractId) -> Self::Data {
        unimplemented!()
    }
}

impl FuelBlockTrait for MockStorage {
    type Error = StorageError;

    fn latest_height(&self) -> Result<BlockHeight, Self::Error> {
        todo!()
    }

    fn block_time(&self, height: &BlockHeight) -> Result<Tai64, Self::Error> {
        todo!()
    }

    fn get_block_id(&self, height: &BlockHeight) -> Result<Option<BlockId>, Self::Error> {
        todo!()
    }
}

impl FuelStateTrait for MockStorage {
    type Error = StorageError;

    fn init_contract_state<S: Iterator<Item=(Bytes32, Bytes32)>>(&mut self, contract_id: &ContractId, slots: S) -> Result<(), Self::Error> {
        todo!()
    }
}

impl DatabaseColumnIterator for MockStorage {
    type Error = StorageError;

    fn iter_all_filtered_column<K, V, P, S>(&self, prefix: Option<P>, start: Option<S>, direction: Option<IterDirection>) -> Box<dyn Iterator<Item=Result<(K, V), Self::Error>> + '_> where K: From<Vec<u8>>, V: serde::de::DeserializeOwned, P: AsRef<[u8]>, S: AsRef<[u8]> {
        todo!()
    }
}