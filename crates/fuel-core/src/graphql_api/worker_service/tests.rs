#![allow(non_snake_case)]

use super::*;
use crate::{
    database::Database,
    graphql_api::storage::relayed_transactions::RelayedTransactionStatuses,
};
use fuel_core_services::{
    stream::IntoBoxStream,
    State,
};
use fuel_core_storage::StorageAsRef;
use fuel_core_types::{
    fuel_tx::Bytes32,
    fuel_types::BlockHeight,
    services::txpool::TransactionStatus,
    tai64::Tai64,
};
use std::sync::Arc;

struct MockTxPool;

impl ports::worker::TxPool for MockTxPool {
    fn send_complete(
        &self,
        _id: Bytes32,
        _block_height: &BlockHeight,
        _status: TransactionStatus,
    ) {
        // Do nothing
    }
}

#[tokio::test]
async fn run__relayed_transaction_events_are_added_to_storage() {
    let tx_id: Bytes32 = [1; 32].into();
    let block_height = 8.into();
    let block_time = Tai64(456);
    let failure = "peanut butter chocolate cake with Kool-Aid".to_string();
    let database = Database::in_memory();
    let (_, receiver) = tokio::sync::watch::channel(State::Started);
    let mut state_watcher = receiver.into();

    // given
    let event = Event::ForcedTransactionFailed {
        id: tx_id.into(),
        block_height,
        block_time,
        failure: failure.clone(),
    };
    let block_importer = block_importer_for_event(event);

    // when
    let mut task =
        worker_task_with_block_importer_and_db(block_importer, database.clone());
    task.run(&mut state_watcher).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // then
    let expected = RelayedTransactionStatus::Failed {
        block_height,
        block_time,
        failure,
    };
    let storage = database.storage_as_ref::<RelayedTransactionStatuses>();
    let actual = storage.get(&tx_id).unwrap().unwrap();
    assert_eq!(*actual, expected);
}

fn block_importer_for_event(event: Event) -> BoxStream<SharedImportResult> {
    let block = Arc::new(ImportResult {
        sealed_block: Default::default(),
        tx_status: vec![],
        events: vec![event],
        source: Default::default(),
    });
    let blocks: Vec<Arc<dyn Deref<Target = ImportResult> + Send + Sync>> = vec![block];
    tokio_stream::iter(blocks).into_boxed()
}

fn worker_task_with_block_importer_and_db<D: ports::worker::Transactional>(
    block_importer: BoxStream<SharedImportResult>,
    database: D,
) -> Task<MockTxPool, D> {
    let tx_pool = MockTxPool;
    let chain_id = Default::default();
    Task {
        tx_pool,
        block_importer,
        database,
        chain_id,
    }
}
