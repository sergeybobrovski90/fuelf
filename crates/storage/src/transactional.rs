//! The primitives to work with storage in transactional mode.

use crate::Result as StorageResult;

/// The types is transactional and may create `StorageTransaction`.
pub trait Transactional<Storage: ?Sized> {
    /// Creates and returns the storage transaction.
    fn transaction(&self) -> StorageTransaction<Storage>;
}

/// The type is storage transaction and holds uncommitted state.
pub trait Transaction<Storage: ?Sized>:
    AsRef<Storage> + AsMut<Storage> + Send + Sync
{
    /// Commits the pending state changes into the storage.
    fn commit(&mut self) -> StorageResult<()>;
}

/// The storage transaction for the `Storage` type.
pub struct StorageTransaction<Storage: ?Sized> {
    transaction: Box<dyn Transaction<Storage>>,
}

impl<Storage> StorageTransaction<Storage> {
    /// Create a new storage transaction.
    pub fn new<T: Transaction<Storage> + 'static>(t: T) -> Self {
        Self {
            transaction: Box::new(t),
        }
    }
}

impl<Storage: ?Sized> Transaction<Storage> for StorageTransaction<Storage> {
    fn commit(&mut self) -> StorageResult<()> {
        self.transaction.commit()
    }
}

impl<Storage: ?Sized + core::fmt::Debug> core::fmt::Debug
    for StorageTransaction<Storage>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StorageTransaction")
            .field("database", &self.transaction.as_ref().as_ref())
            .finish()
    }
}

impl<Storage: ?Sized> AsRef<Storage> for StorageTransaction<Storage> {
    fn as_ref(&self) -> &Storage {
        (*self.transaction).as_ref()
    }
}

impl<Storage: ?Sized> AsMut<Storage> for StorageTransaction<Storage> {
    fn as_mut(&mut self) -> &mut Storage {
        (*self.transaction).as_mut()
    }
}

impl<Storage: ?Sized> StorageTransaction<Storage> {
    /// Committing of the state consumes `Self`.
    pub fn commit(mut self) -> StorageResult<()> {
        self.transaction.commit()
    }
}
