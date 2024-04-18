use anyhow::bail;
use fuel_core_chain_config::TableEntry;
use fuel_core_storage::{
    structured_storage::TableWithBlueprint,
    transactional::{
        Modifiable,
        StorageTransaction,
        WriteTransaction,
    },
    StorageAsRef,
    StorageInspect,
    StorageMutate,
};

use crate::{
    database::{
        database_description::DatabaseDescription,
        genesis_progress::{
            GenesisMetadata,
            GenesisProgressMutate,
        },
        Database,
    },
    service::genesis::{
        importer::migration_name,
        task_manager::CancellationToken,
    },
};

use super::progress::ProgressReporter;

pub struct ImportTask<Handler, Groups, DbDesc>
where
    DbDesc: DatabaseDescription,
{
    handler: Handler,
    skip: usize,
    groups: Groups,
    cancel_token: CancellationToken,
    db: Database<DbDesc>,
    reporter: ProgressReporter,
}

pub trait ImportTable {
    type TableInSnapshot: TableWithBlueprint;
    type TableBeingWritten: TableWithBlueprint;
    type DbDesc: DatabaseDescription;

    fn process(
        &mut self,
        group: Vec<TableEntry<Self::TableInSnapshot>>,
        tx: &mut StorageTransaction<&mut Database<Self::DbDesc>>,
    ) -> anyhow::Result<()>;
}

impl<Logic, GroupGenerator, DbDesc> ImportTask<Logic, GroupGenerator, DbDesc>
where
    DbDesc: DatabaseDescription,
    Logic: ImportTable<DbDesc = DbDesc>,
    Database<DbDesc>: StorageInspect<GenesisMetadata<DbDesc>>,
{
    pub fn new(
        cancel_token: CancellationToken,
        handler: Logic,
        groups: GroupGenerator,
        db: Database<DbDesc>,
        reporter: ProgressReporter,
    ) -> Self {
        let progress_name =
            migration_name::<Logic::TableInSnapshot, Logic::TableBeingWritten>();
        let skip = match db
            .storage::<GenesisMetadata<DbDesc>>()
            .get(progress_name.as_str())
        {
            Ok(Some(idx_last_handled)) => {
                usize::saturating_add(idx_last_handled.into_owned(), 1)
            }
            _ => 0,
        };

        Self {
            handler,
            skip,
            groups,
            cancel_token,
            db,
            reporter,
        }
    }
}

impl<Logic, GroupGenerator, DbDesc> ImportTask<Logic, GroupGenerator, DbDesc>
where
    DbDesc: DatabaseDescription,
    Logic: ImportTable<DbDesc = DbDesc>,
    GroupGenerator:
        IntoIterator<Item = anyhow::Result<Vec<TableEntry<Logic::TableInSnapshot>>>>,
    GenesisMetadata<DbDesc>: TableWithBlueprint<
        Column = DbDesc::Column,
        Key = str,
        Value = usize,
        OwnedValue = usize,
    >,
    Database<DbDesc>:
        StorageInspect<GenesisMetadata<DbDesc>> + WriteTransaction + Modifiable,
    for<'a> StorageTransaction<&'a mut Database<DbDesc>>:
        StorageMutate<GenesisMetadata<DbDesc>, Error = fuel_core_storage::Error>,
{
    pub fn run(mut self) -> anyhow::Result<()> {
        let mut db = self.db;
        let mut is_cancelled = self.cancel_token.is_cancelled();
        self.groups
            .into_iter()
            .enumerate()
            .skip(self.skip)
            .take_while(|_| {
                is_cancelled = self.cancel_token.is_cancelled();
                !is_cancelled
            })
            .try_for_each(|(index, group)| {
                let group = group?;
                let mut tx = db.write_transaction();
                self.handler.process(group, &mut tx)?;

                let progress_name =
                    migration_name::<Logic::TableInSnapshot, Logic::TableBeingWritten>();

                GenesisProgressMutate::<DbDesc>::update_genesis_progress(
                    &mut tx,
                    progress_name.as_str(),
                    index,
                )?;
                tx.commit()?;
                self.reporter
                    .set_progress(u64::try_from(index).unwrap_or(u64::MAX));
                anyhow::Result::<_>::Ok(())
            })?;

        if is_cancelled {
            bail!("Import cancelled")
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        database::genesis_progress::GenesisProgressInspect,
        service::genesis::{
            importer::{
                import_task::ImportTask,
                progress::ProgressReporter,
            },
            task_manager::CancellationToken,
        },
    };
    use std::sync::{
        Arc,
        Mutex,
    };

    use anyhow::{
        anyhow,
        bail,
    };
    use fuel_core_chain_config::{
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
            Value,
        },
        tables::Coins,
        transactional::{
            Changes,
            StorageTransaction,
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

    use crate::{
        combined_database::CombinedDatabase,
        database::{
            database_description::on_chain::OnChain,
            genesis_progress::GenesisProgressMutate,
            Database,
        },
        service::genesis::importer::migration_name,
        state::{
            in_memory::memory_store::MemoryStore,
            TransactableStorage,
        },
    };

    use super::ImportTable;

    struct TestHandler<L> {
        logic: L,
    }

    impl<L> TestHandler<L>
    where
        TestHandler<L>: ImportTable,
    {
        pub fn new(logic: L) -> Self {
            Self { logic }
        }
    }

    impl<L> ImportTable for TestHandler<L>
    where
        L: FnMut(
            TableEntry<Coins>,
            &mut StorageTransaction<&mut Database>,
        ) -> anyhow::Result<()>,
    {
        type TableInSnapshot = Coins;
        type TableBeingWritten = Coins;
        type DbDesc = OnChain;
        fn process(
            &mut self,
            group: Vec<TableEntry<Self::TableInSnapshot>>,
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

        pub fn as_groups(&self) -> Vec<Vec<TableEntry<Coins>>> {
            self.batches.clone()
        }

        pub fn as_ok_groups(&self) -> Vec<anyhow::Result<Vec<TableEntry<Coins>>>> {
            self.as_groups().into_iter().map(Ok).collect()
        }
    }

    #[test]
    fn will_go_through_all_groups() {
        // given
        let data = TestData::new(3);

        let mut called_with = vec![];
        let runner = ImportTask::new(
            CancellationToken::default(),
            TestHandler::new(|group, _| {
                called_with.push(group);
                Ok(())
            }),
            data.as_ok_groups(),
            Database::default(),
            ProgressReporter::default(),
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
        let mut db = CombinedDatabase::default();
        GenesisProgressMutate::<OnChain>::update_genesis_progress(
            db.on_chain_mut(),
            migration_name::<Coins, Coins>().as_str(),
            0,
        )
        .unwrap();
        let runner = ImportTask::new(
            CancellationToken::default(),
            TestHandler::new(|element, _| {
                called_with.push(element);
                Ok(())
            }),
            data.as_ok_groups(),
            db.on_chain().clone(),
            ProgressReporter::default(),
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

        let runner = ImportTask::new(
            CancellationToken::default(),
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
            ProgressReporter::default(),
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

        let runner = ImportTask::new(
            CancellationToken::default(),
            TestHandler::new(|_, tx| {
                insert_a_coin(tx, &utxo_id);
                bail!("Some error")
            }),
            groups.as_ok_groups(),
            db.clone(),
            ProgressReporter::default(),
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
        let runner = ImportTask::new(
            CancellationToken::default(),
            TestHandler::new(|_, _| bail!("Some error")),
            groups.as_ok_groups(),
            Database::default(),
            ProgressReporter::default(),
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
        let runner = ImportTask::new(
            CancellationToken::default(),
            TestHandler::new(|_, _| Ok(())),
            groups,
            Database::default(),
            ProgressReporter::default(),
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
        let runner = ImportTask::new(
            CancellationToken::default(),
            TestHandler::new(|_, _| Ok(())),
            data.as_ok_groups(),
            db.clone(),
            ProgressReporter::default(),
        );

        // when
        runner.run().unwrap();

        // then
        assert_eq!(
            GenesisProgressInspect::<OnChain>::genesis_progress(
                &db,
                migration_name::<Coins, Coins>().as_str(),
            ),
            Some(1)
        );
    }

    #[tokio::test]
    async fn processing_stops_when_cancelled() {
        // given
        let (tx, rx) = std::sync::mpsc::channel();

        let read_groups = Arc::new(Mutex::new(vec![]));
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let runner = {
            let read_groups = Arc::clone(&read_groups);
            ImportTask::new(
                cancel_token.clone().into(),
                TestHandler::new(move |el, _| {
                    read_groups.lock().unwrap().push(el);
                    Ok(())
                }),
                rx,
                Database::default(),
                ProgressReporter::default(),
            )
        };

        let runner_handle = std::thread::spawn(move || runner.run());

        let data = TestData::new(4);
        let take = 3;
        for group in data.as_ok_groups().into_iter().take(take) {
            tx.send(group).unwrap();
        }

        while read_groups.lock().unwrap().len() < take {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        cancel_token.cancel();

        // when
        tx.send(data.as_ok_groups().pop().unwrap()).unwrap();

        // then
        // runner should finish
        drop(tx);
        runner_handle
            .join()
            .unwrap()
            .expect_err("Cancelling is an error");

        // group after signal is not read
        let read_entries = read_groups.lock().unwrap().clone();
        let inserted_groups = data
            .as_entries(0)
            .into_iter()
            .take(take)
            .collect::<Vec<_>>();
        assert_eq!(read_entries, inserted_groups);
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
        let runner = ImportTask::new(
            CancellationToken::default(),
            TestHandler::new(|_, _| Ok(())),
            groups.as_ok_groups(),
            Database::new(Arc::new(BrokenTransactions::new())),
            ProgressReporter::default(),
        );

        // when
        let result = runner.run();

        // then
        assert!(result.is_err());
    }
}
