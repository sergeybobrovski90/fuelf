use fuel_core_chain_config::{
    Group,
    TableEntry,
};
use fuel_core_storage::{
    kv_store::StorageColumn,
    structured_storage::TableWithBlueprint,
    transactional::{
        StorageTransaction,
        WriteTransaction,
    },
};
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::database::{
    genesis_progress::{
        GenesisProgressInspect,
        GenesisProgressMutate,
    },
    Database,
};

pub trait TransactionOpener {
    fn transaction(&mut self) -> StorageTransaction<&mut Database>;

    fn view_only(&self) -> &Database;
}

impl TransactionOpener for Database {
    fn transaction(&mut self) -> StorageTransaction<&mut Database> {
        self.write_transaction()
    }

    fn view_only(&self) -> &Database {
        self
    }
}

pub struct GenesisRunner<Handler, Groups, TxOpener> {
    handler: Handler,
    tx_opener: TxOpener,
    skip: usize,
    groups: Groups,
    finished_signal: Option<Arc<Notify>>,
    cancel_token: CancellationToken,
}

pub trait ProcessState {
    type Table: TableWithBlueprint;

    fn process(
        &mut self,
        group: Vec<TableEntry<Self::Table>>,
        tx: &mut StorageTransaction<&mut Database>,
    ) -> anyhow::Result<()>;
}

impl<Logic, GroupGenerator, TxOpener> GenesisRunner<Logic, GroupGenerator, TxOpener>
where
    Logic: ProcessState,
    GroupGenerator: IntoIterator<Item = anyhow::Result<Group<TableEntry<Logic::Table>>>>,
    TxOpener: TransactionOpener,
{
    pub fn new(
        finished_signal: Option<Arc<Notify>>,
        cancel_token: CancellationToken,
        handler: Logic,
        groups: GroupGenerator,
        tx_opener: TxOpener,
    ) -> Self {
        let skip = tx_opener
            .view_only()
            .genesis_progress(Logic::Table::column().name())
            // The `idx_last_handled` is zero based, so we need to add 1 to skip the already handled groups.
            .map(|idx_last_handled| idx_last_handled.saturating_add(1))
            .unwrap_or_default();
        Self {
            handler,
            skip,
            groups,
            tx_opener,
            finished_signal,
            cancel_token,
        }
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        let result = self
            .groups
            .into_iter()
            .skip(self.skip)
            .take_while(|_| !self.cancel_token.is_cancelled())
            .try_for_each(|group| {
                let mut tx = self.tx_opener.transaction();
                let group = group?;
                let group_num = group.index;
                self.handler.process(group.data, &mut tx)?;
                tx.update_genesis_progress(Logic::Table::column().name(), group_num)?;
                tx.commit()?;
                Ok(())
            });

        if let Some(finished_signal) = &self.finished_signal {
            finished_signal.notify_one();
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            Mutex,
        },
        time::Duration,
    };

    use anyhow::{
        anyhow,
        bail,
    };
    use fuel_core_chain_config::{
        Group,
        Randomize,
        TableEntry,
    };
    use fuel_core_storage::{
        column::Column,
        iter::{
            BoxedIter,
            IterDirection,
            IterableStore,
        },
        kv_store::{
            KVItem,
            KeyValueInspect,
            StorageColumn,
            Value,
        },
        structured_storage::TableWithBlueprint,
        tables::Coins,
        transactional::{
            Changes,
            StorageTransaction,
            WriteTransaction,
        },
        Result as StorageResult,
        StorageAsMut,
        StorageAsRef,
        StorageInspect,
    };
    use fuel_core_types::{
        entities::coins::coin::{
            CompressedCoin,
            CompressedCoinV1,
        },
        fuel_tx::UtxoId,
        fuel_types::BlockHeight,
    };
    use rand::{
        rngs::StdRng,
        SeedableRng,
    };
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use crate::{
        database::{
            genesis_progress::{
                GenesisProgressInspect,
                GenesisProgressMutate,
            },
            Database,
        },
        service::genesis::runner::{
            GenesisRunner,
            TransactionOpener,
        },
        state::{
            in_memory::memory_store::MemoryStore,
            TransactableStorage,
        },
    };

    use super::ProcessState;

    struct TestHandler<L> {
        logic: L,
    }

    impl<L> TestHandler<L>
    where
        TestHandler<L>: ProcessState,
    {
        pub fn new(logic: L) -> Self {
            Self { logic }
        }
    }

    impl<L> ProcessState for TestHandler<L>
    where
        L: FnMut(
            TableEntry<Coins>,
            &mut StorageTransaction<&mut Database>,
        ) -> anyhow::Result<()>,
    {
        type Table = Coins;
        fn process(
            &mut self,
            group: Vec<TableEntry<Self::Table>>,
            tx: &mut StorageTransaction<&mut Database>,
        ) -> anyhow::Result<()> {
            group
                .into_iter()
                .try_for_each(|item| (self.logic)(item, tx))
        }
    }

    struct TestData {
        batches: Vec<Vec<TableEntry<Coins>>>,
    }

    impl TestData {
        pub fn new(amount: usize) -> Self {
            let mut rng = StdRng::seed_from_u64(0);
            let batches = std::iter::repeat_with(|| TableEntry::randomize(&mut rng))
                .take(amount)
                .map(|el| vec![el])
                .collect();
            Self { batches }
        }

        pub fn as_entries(&self, skip_batches: usize) -> Vec<TableEntry<Coins>> {
            self.batches
                .iter()
                .skip(skip_batches)
                .flat_map(|batch| batch.clone())
                .collect()
        }

        pub fn as_indexed_groups(&self) -> Vec<Group<TableEntry<Coins>>> {
            self.batches
                .iter()
                .enumerate()
                .map(|(index, data)| Group {
                    index,
                    data: data.clone(),
                })
                .collect()
        }

        pub fn as_ok_groups(&self) -> Vec<anyhow::Result<Group<TableEntry<Coins>>>> {
            self.as_indexed_groups().into_iter().map(Ok).collect()
        }
    }

    #[test]
    fn will_go_through_all_groups() {
        // given
        let data = TestData::new(3);

        let mut called_with = vec![];
        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|group, _| {
                called_with.push(group);
                Ok(())
            }),
            data.as_ok_groups(),
            Database::default(),
        );

        // when
        runner.run().unwrap();

        // then
        assert_eq!(called_with, data.as_entries(0));
    }

    #[test]
    fn will_skip_one_group() {
        // given
        let data = TestData::new(2);

        let mut called_with = vec![];
        let mut db = Database::default();
        db.update_genesis_progress(Coins::column().name(), 0)
            .unwrap();

        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|element, _| {
                called_with.push(element);
                Ok(())
            }),
            data.as_ok_groups(),
            db,
        );

        // when
        runner.run().unwrap();

        // then
        assert_eq!(called_with, data.as_entries(1));
    }

    #[test]
    fn changes_to_db_by_handler_are_behind_a_transaction() {
        // given
        let groups = TestData::new(1);
        let outer_db = Database::default();
        let utxo_id = UtxoId::new(Default::default(), 0);

        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|_, tx| {
                insert_a_coin(tx, &utxo_id);

                assert!(
                    tx.storage::<Coins>().contains_key(&utxo_id).unwrap(),
                    "Coin should be present in the tx db view"
                );

                assert!(
                    !outer_db
                        .storage_as_ref::<Coins>()
                        .contains_key(&utxo_id)
                        .unwrap(),
                    "Coin should not be present in the outer db "
                );

                Ok(())
            }),
            groups.as_ok_groups(),
            outer_db.clone(),
        );

        // when
        runner.run().unwrap();

        // then
        assert!(outer_db
            .storage_as_ref::<Coins>()
            .contains_key(&utxo_id)
            .unwrap());
    }

    fn insert_a_coin(tx: &mut StorageTransaction<&mut Database>, utxo_id: &UtxoId) {
        let coin: CompressedCoin = CompressedCoinV1::default().into();

        tx.storage_as_mut::<Coins>().insert(utxo_id, &coin).unwrap();
    }

    #[test]
    fn tx_reverted_if_handler_fails() {
        // given
        let groups = TestData::new(1);
        let db = Database::default();
        let utxo_id = UtxoId::new(Default::default(), 0);

        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|_, tx| {
                insert_a_coin(tx, &utxo_id);
                bail!("Some error")
            }),
            groups.as_ok_groups(),
            db.clone(),
        );

        // when
        let _ = runner.run();

        // then
        assert!(!StorageInspect::<Coins>::contains_key(&db, &utxo_id).unwrap());
    }

    #[test]
    fn handler_failure_is_propagated() {
        // given
        let groups = TestData::new(1);
        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|_, _| bail!("Some error")),
            groups.as_ok_groups(),
            Database::default(),
        );

        // when
        let result = runner.run();

        // then
        assert!(result.is_err());
    }

    #[test]
    fn seeing_an_invalid_group_propagates_the_error() {
        // given
        let groups = [Err(anyhow!("Some error"))];
        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|_, _| Ok(())),
            groups,
            Database::default(),
        );

        // when
        let result = runner.run();

        // then
        assert!(result.is_err());
    }

    #[test]
    fn succesfully_processed_batch_updates_the_genesis_progress() {
        // given
        let data = TestData::new(2);
        let db = Database::default();
        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|_, _| Ok(())),
            data.as_ok_groups(),
            db.clone(),
        );

        // when
        runner.run().unwrap();

        // then
        assert_eq!(db.genesis_progress(Coins::column().name()), Some(1));
    }

    #[test]
    fn genesis_progress_is_increased_in_same_transaction_as_batch_work() {
        struct OnlyOneTransactionAllowed {
            db: Database,
            counter: usize,
        }
        impl TransactionOpener for OnlyOneTransactionAllowed {
            fn transaction(&mut self) -> StorageTransaction<&mut Database> {
                if self.counter == 0 {
                    self.counter += 1;
                    self.db.write_transaction()
                } else {
                    panic!("Only one transaction should be opened")
                }
            }

            fn view_only(&self) -> &Database {
                &self.db
            }
        }

        // given
        let data = TestData::new(1);
        let db = Database::default();
        let tx_opener = OnlyOneTransactionAllowed {
            db: db.clone(),
            counter: 0,
        };
        let utxo_id = UtxoId::new(Default::default(), 0);

        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|_, tx| {
                insert_a_coin(tx, &utxo_id);
                Ok(())
            }),
            data.as_ok_groups(),
            tx_opener,
        );

        // when
        runner.run().unwrap();

        // then
        assert_eq!(db.genesis_progress(Coins::column().name()), Some(0));
        assert!(db.storage_as_ref::<Coins>().contains_key(&utxo_id).unwrap());
    }

    #[tokio::test]
    async fn processing_stops_when_cancelled() {
        // given
        let finished_signal = Arc::new(Notify::new());
        let cancel_token = CancellationToken::new();

        let (tx, rx) = std::sync::mpsc::channel();

        let read_groups = Arc::new(Mutex::new(vec![]));
        let runner = {
            let read_groups = Arc::clone(&read_groups);
            GenesisRunner::new(
                Some(Arc::clone(&finished_signal)),
                cancel_token.clone(),
                TestHandler::new(move |el, _| {
                    read_groups.lock().unwrap().push(el);
                    Ok(())
                }),
                rx,
                Database::default(),
            )
        };

        let runner_handle = std::thread::spawn(move || runner.run());

        let data = TestData::new(4);
        for group in data.as_ok_groups() {
            tx.send(group).unwrap();
        }

        while read_groups.lock().unwrap().len() < 3 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        cancel_token.cancel();

        // when
        tx.send(data.as_ok_groups().pop().unwrap()).unwrap();

        // then
        // runner should finish
        drop(tx);
        let runner_response = runner_handle.join().unwrap();
        assert!(
            runner_response.is_ok(),
            "Stopping a runner should not be an error"
        );

        // group after signal is not read
        let read_entries = read_groups.lock().unwrap().clone();
        let inserted_groups = data.as_entries(0);
        assert_eq!(read_entries, inserted_groups);

        // finished signal is emitted
        tokio::time::timeout(Duration::from_millis(10), finished_signal.notified())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn emits_finished_signal_on_error() {
        // given
        let finished_signal = Arc::new(Notify::new());
        let groups = [Err(anyhow!("Some error"))];
        let runner = GenesisRunner::new(
            Some(Arc::clone(&finished_signal)),
            CancellationToken::new(),
            TestHandler::new(|_, _| Ok(())),
            groups,
            Database::default(),
        );

        // when
        let result = runner.run();

        // then
        assert!(result.is_err());
        tokio::time::timeout(Duration::from_millis(10), finished_signal.notified())
            .await
            .unwrap();
    }

    #[derive(Debug)]
    struct BrokenTransactions {
        store: MemoryStore,
    }

    impl BrokenTransactions {
        fn new() -> Self {
            Self {
                store: MemoryStore::default(),
            }
        }
    }

    impl KeyValueInspect for BrokenTransactions {
        type Column = Column;

        fn get(&self, key: &[u8], column: Column) -> StorageResult<Option<Value>> {
            self.store.get(key, column)
        }
    }

    impl IterableStore for BrokenTransactions {
        fn iter_store(
            &self,
            _: Self::Column,
            _: Option<&[u8]>,
            _: Option<&[u8]>,
            _: IterDirection,
        ) -> BoxedIter<KVItem> {
            unimplemented!()
        }
    }

    impl TransactableStorage<BlockHeight> for BrokenTransactions {
        fn commit_changes(
            &self,
            _: Option<BlockHeight>,
            _: Changes,
        ) -> StorageResult<()> {
            Err(anyhow::anyhow!("I refuse to work!").into())
        }
    }

    #[test]
    fn tx_commit_failure_is_propagated() {
        // given
        let groups = TestData::new(1);
        let runner = GenesisRunner::new(
            Some(Arc::new(Notify::new())),
            CancellationToken::new(),
            TestHandler::new(|_, _| Ok(())),
            groups.as_ok_groups(),
            Database::new(Arc::new(BrokenTransactions::new())),
        );

        // when
        let result = runner.run();

        // then
        assert!(result.is_err());
    }
}
