use crate::state::Error::Codec;
use crate::state::{
    in_memory::transaction::MemoryTransactionView, BatchOperations, Error, KeyValueStore, Result, Transactional,
    WriteOperation,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug)]
pub struct MemoryStore<K, V> {
    inner: Arc<RwLock<HashMap<Vec<u8>, Vec<u8>>>>,
    _key_marker: PhantomData<K>,
    _value_marker: PhantomData<V>,
}

impl<K, V> MemoryStore<K, V> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::<Vec<u8>, Vec<u8>>::new())),
            _key_marker: PhantomData,
            _value_marker: PhantomData,
        }
    }
}

impl<K, V> KeyValueStore for MemoryStore<K, V>
where
    K: AsRef<[u8]> + Into<Vec<u8>> + Debug + Clone,
    V: Serialize + DeserializeOwned + Debug + Clone,
{
    type Key = K;
    type Value = V;

    fn get(&self, key: Self::Key) -> Result<Option<Self::Value>> {
        if let Some(value) = self.inner.read().expect("poisoned").get(key.as_ref()) {
            Ok(Some(bincode::deserialize(value).map_err(|_| Codec)?))
        } else {
            Ok(None)
        }
    }

    fn put(&mut self, key: Self::Key, value: Self::Value) -> Result<Option<Self::Value>> {
        let value = bincode::serialize(&value).unwrap();
        let result = self.inner.write().expect("poisoned").insert(key.into(), value);
        if let Some(previous) = result {
            Ok(Some(bincode::deserialize(&previous).map_err(|_| Error::Codec)?))
        } else {
            Ok(None)
        }
    }

    fn delete(&mut self, key: Self::Key) -> Result<Option<Self::Value>> {
        Ok(self
            .inner
            .write()
            .expect("poisoned")
            .remove(key.as_ref())
            .map(|res| bincode::deserialize(&res).unwrap()))
    }

    fn exists(&self, key: Self::Key) -> Result<bool> {
        Ok(self.inner.read().expect("poisoned").contains_key(key.as_ref()))
    }
}

impl<K, V> BatchOperations for MemoryStore<K, V>
where
    K: AsRef<[u8]> + Into<Vec<u8>> + Debug + Clone,
    V: Serialize + DeserializeOwned + Debug + Clone,
{
    type Key = K;
    type Value = V;

    fn batch_write<I>(&mut self, entries: I) -> Result<()>
    where
        I: Iterator<Item = WriteOperation<Self::Key, Self::Value>>,
    {
        for entry in entries {
            match entry {
                WriteOperation::Insert(key, value) => {
                    let _ = self.put(key, value);
                }
                WriteOperation::Remove(key) => {
                    let _ = self.delete(key);
                }
            }
        }
        Ok(())
    }
}

/// Configure memory store to use the MemoryTransactionView
impl<K, V> Transactional<K, V> for MemoryStore<K, V>
where
    K: AsRef<[u8]> + Into<Vec<u8>> + Debug + Clone,
    V: Serialize + DeserializeOwned + Debug + Clone,
{
    type View = MemoryTransactionView<K, V, Self>;
}
