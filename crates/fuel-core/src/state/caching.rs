use crate::{
    database::{
        Column,
        Result as DatabaseResult,
    },
    state::{
        BatchOperations,
        DataSource,
        IterDirection,
        KVItem,
        KeyValueStore,
        TransactableStorage,
        Value,
        WriteOperation,
    },
};
use fuel_core_storage::iter::BoxedIter;
use std::{
    fmt::Debug,
    time::Duration,
};

type MokaCache = moka::sync::Cache<Vec<u8>, Option<Value>>;

#[derive(Debug, Clone)]
pub struct Cache {
    inner: [MokaCache; Column::COUNT],
    data_source: DataSource,
}

impl Cache {
    pub fn new(data_source: DataSource, max_capacity: u64) -> Self {
        Self {
            inner: array_init::array_init(|_| {
                moka::sync::CacheBuilder::new(max_capacity)
                    .time_to_idle(Duration::from_secs(5 * 60))
                    .build()
            }),
            data_source,
        }
    }
}

impl KeyValueStore for Cache {
    fn get(&self, key: &[u8], column: Column) -> DatabaseResult<Option<Value>> {
        let vec_key = key.to_vec();
        if let Some(value) = self.inner[column.as_usize()].get(&vec_key) {
            Ok(value)
        } else {
            let value = self.data_source.get(key, column)?;

            self.inner[column.as_usize()].insert(vec_key, value.clone());
            Ok(value)
        }
    }

    fn put(
        &self,
        key: &[u8],
        column: Column,
        value: Value,
    ) -> DatabaseResult<Option<Value>> {
        let vec_key = key.to_vec();
        self.inner[column.as_usize()].insert(vec_key, Some(value.clone()));
        self.data_source.put(key, column, value)
    }

    fn delete(&self, key: &[u8], column: Column) -> DatabaseResult<Option<Value>> {
        let vec_key = key.to_vec();
        self.inner[column.as_usize()].insert(vec_key, None);
        self.data_source.delete(key, column)
    }

    fn exists(&self, key: &[u8], column: Column) -> DatabaseResult<bool> {
        let vec_key = key.to_vec();
        if let Some(value) = self.inner[column.as_usize()].get(&vec_key) {
            Ok(value.is_some())
        } else {
            let value = self.data_source.get(key, column)?;

            self.inner[column.as_usize()].insert(vec_key, value.clone());
            Ok(value.is_some())
        }
    }

    fn iter_all(
        &self,
        column: Column,
        prefix: Option<&[u8]>,
        start: Option<&[u8]>,
        direction: IterDirection,
    ) -> BoxedIter<KVItem> {
        // Don't optimize iteration
        self.data_source.iter_all(column, prefix, start, direction)
    }
}

impl BatchOperations for Cache {
    fn batch_write(
        &self,
        entries: &mut dyn Iterator<Item = (Vec<u8>, Column, WriteOperation)>,
    ) -> DatabaseResult<()> {
        let mut entries = entries.map(|(key, column, op)| {
            let op = match op {
                WriteOperation::Insert(value) => {
                    self.inner[column.as_usize()]
                        .insert(key.clone(), Some(value.clone()));
                    WriteOperation::Insert(value)
                }
                WriteOperation::Remove => {
                    self.inner[column.as_usize()].insert(key.clone(), None);
                    WriteOperation::Remove
                }
            };
            (key, column, op)
        });
        self.data_source.batch_write(&mut entries)
    }
}

impl TransactableStorage for Cache {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::in_memory::memory_store::MemoryStore;
    use std::sync::Arc;

    #[test]
    fn get_returns_from_cache() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let cache = Cache::new(store, 100);
        let key = vec![0xA, 0xB, 0xC];
        cache
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        // test
        let ret = cache.get(&key, Column::Metadata).unwrap();
        // verify
        assert_eq!(ret, Some(Arc::new(vec![1, 2, 3])))
    }

    #[test]
    fn get_returns_from_data_store_when_key_not_in_cache() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let key = vec![0xA, 0xB, 0xC];
        store
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        let cache = Cache::new(store, 100);
        // test
        let ret = cache.get(&key, Column::Metadata).unwrap();
        // verify
        assert_eq!(ret, Some(Arc::new(vec![1, 2, 3])))
    }

    #[test]
    fn get_does_not_return_removed_element() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let key = vec![0xA, 0xB, 0xC];
        store
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        let cache = Cache::new(store.clone(), 100);
        cache.delete(&key, Column::Metadata).unwrap();
        // test
        let ret = cache.get(&key, Column::Metadata).unwrap();
        let original = store.get(&key, Column::Metadata).unwrap();
        // verify
        assert_eq!(ret, None);
        // also ensure the original value is removed too
        assert_eq!(original, None)
    }

    #[test]
    fn can_insert_value_into_cache() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let cache = Cache::new(store, 100);
        let _ = cache.put(&[0xA, 0xB, 0xC], Column::Metadata, Arc::new(vec![1, 2, 3]));
        // test
        let ret = cache
            .put(&[0xA, 0xB, 0xC], Column::Metadata, Arc::new(vec![2, 4, 6]))
            .unwrap();
        // verify
        assert_eq!(ret, Some(Arc::new(vec![1, 2, 3])))
    }

    #[test]
    fn delete_value_from_cache_returns_value() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let cache = Cache::new(store, 100);
        let key = vec![0xA, 0xB, 0xC];
        cache
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        // test
        let ret = cache.delete(&key, Column::Metadata).unwrap();
        let get = cache.get(&key, Column::Metadata).unwrap();
        // verify
        assert_eq!(ret, Some(Arc::new(vec![1, 2, 3])));
        assert_eq!(get, None)
    }

    #[test]
    fn delete_returns_datastore_value_when_not_in_cache() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let key = vec![0xA, 0xB, 0xC];
        store
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        let cache = Cache::new(store, 100);
        // test
        let ret = cache.delete(&key, Column::Metadata).unwrap();
        let get = cache.get(&key, Column::Metadata).unwrap();
        // verify
        assert_eq!(ret, Some(Arc::new(vec![1, 2, 3])));
        assert_eq!(get, None)
    }

    #[test]
    fn delete_does_not_return_datastore_value_when_deleted_twice() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let key = vec![0xA, 0xB, 0xC];
        store
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        let cache = Cache::new(store, 100);
        // test
        let ret1 = cache.delete(&key, Column::Metadata).unwrap();
        let ret2 = cache.delete(&key, Column::Metadata).unwrap();
        // verify
        assert_eq!(ret1, Some(Arc::new(vec![1, 2, 3])));
        assert_eq!(ret2, None);
    }

    #[test]
    fn exists_checks_chache_values() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let cache = Cache::new(store, 100);
        let key = vec![0xA, 0xB, 0xC];
        cache
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        // test
        let ret = cache.exists(&key, Column::Metadata).unwrap();
        // verify
        assert!(ret)
    }

    #[test]
    fn exists_checks_data_store_when_not_in_cache() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let key = vec![0xA, 0xB, 0xC];
        store
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        let cache = Cache::new(store, 100);
        // test
        let ret = cache.exists(&key, Column::Metadata).unwrap();
        // verify
        assert!(ret)
    }

    #[test]
    fn exists_is_false_after_removal() {
        // setup
        let store = Arc::new(MemoryStore::default());
        let key = vec![0xA, 0xB, 0xC];
        store
            .put(&key, Column::Metadata, Arc::new(vec![1, 2, 3]))
            .unwrap();
        let cache = Cache::new(store.clone(), 100);
        cache.delete(&key, Column::Metadata).unwrap();
        // test
        let ret = cache.exists(&key, Column::Metadata).unwrap();
        let original = store.exists(&key, Column::Metadata).unwrap();
        // verify
        assert!(!ret);
        // also ensure the original value is updated
        assert!(!original)
    }

    #[test]
    fn can_use_unit_value() {
        let key = vec![0x00];

        let store = Arc::new(MemoryStore::default());
        let db = Cache::new(store, 100);
        let expected = Arc::new(vec![]);
        db.put(&key, Column::Metadata, expected.clone()).unwrap();

        assert_eq!(db.get(&key, Column::Metadata).unwrap().unwrap(), expected);

        assert!(db.exists(&key, Column::Metadata).unwrap());

        assert_eq!(
            db.iter_all(Column::Metadata, None, None, IterDirection::Forward)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![(key.clone(), expected.clone())]
        );

        assert_eq!(
            db.delete(&key, Column::Metadata).unwrap().unwrap(),
            expected
        );

        assert!(!db.exists(&key, Column::Metadata).unwrap());
    }

    #[test]
    fn can_use_unit_key() {
        let key: Vec<u8> = Vec::with_capacity(0);

        let store = Arc::new(MemoryStore::default());
        let db = Cache::new(store, 100);
        let expected = Arc::new(vec![1, 2, 3]);
        db.put(&key, Column::Metadata, expected.clone()).unwrap();

        assert_eq!(db.get(&key, Column::Metadata).unwrap().unwrap(), expected);

        assert!(db.exists(&key, Column::Metadata).unwrap());

        assert_eq!(
            db.iter_all(Column::Metadata, None, None, IterDirection::Forward)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![(key.clone(), expected.clone())]
        );

        assert_eq!(
            db.delete(&key, Column::Metadata).unwrap().unwrap(),
            expected
        );

        assert!(!db.exists(&key, Column::Metadata).unwrap());
    }

    #[test]
    fn can_use_unit_key_and_value() {
        let key: Vec<u8> = Vec::with_capacity(0);

        let store = Arc::new(MemoryStore::default());
        let db = Cache::new(store, 100);
        let expected = Arc::new(vec![]);
        db.put(&key, Column::Metadata, expected.clone()).unwrap();

        assert_eq!(db.get(&key, Column::Metadata).unwrap().unwrap(), expected);

        assert!(db.exists(&key, Column::Metadata).unwrap());

        assert_eq!(
            db.iter_all(Column::Metadata, None, None, IterDirection::Forward)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![(key.clone(), expected.clone())]
        );

        assert_eq!(
            db.delete(&key, Column::Metadata).unwrap().unwrap(),
            expected
        );

        assert!(!db.exists(&key, Column::Metadata).unwrap());
    }
}
