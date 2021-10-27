use chrono::{TimeZone, Utc};
use fuel_client::client::FuelClient;
use fuel_core::database::Database;
use fuel_core::{
    model::fuel_block::FuelBlock,
    schema::scalars::HexString256,
    service::{configure, run_in_background},
};
use fuel_storage::Storage;
use fuel_vm::prelude::Bytes32;
use itertools::{rev, Itertools};

#[tokio::test]
async fn block() {
    // setup test data in the node
    let block = FuelBlock::default();
    let id = block.id();
    let mut db = Database::default();
    Storage::<Bytes32, FuelBlock>::insert(&mut db, &id, &block).unwrap();

    // setup server & client
    let srv = run_in_background(configure(db)).await;
    let client = FuelClient::from(srv);

    // run test
    let block = client
        .block(HexString256::from(id).to_string().as_str())
        .await
        .unwrap();
    assert!(block.is_some());
}

#[tokio::test]
async fn block_connection_first_5() {
    // blocks
    let blocks = (0..10u32)
        .map(|i| FuelBlock {
            fuel_height: i.into(),
            transactions: vec![],
            time: Utc.timestamp(i.into(), 0),
            producer: Default::default(),
        })
        .collect_vec();

    // setup test data in the node
    let mut db = Database::default();
    for block in blocks {
        let id = block.id();
        Storage::<Bytes32, FuelBlock>::insert(&mut db, &id, &block).unwrap();
    }

    // setup server & client
    let srv = run_in_background(configure(db)).await;
    let client = FuelClient::from(srv);

    // run test
    let blocks = client.blocks(Some(5), None, None, None).await.unwrap();
    assert!(blocks.edges.is_some());
    // assert "first" 5 blocks are returned in descending order (latest first)
    assert_eq!(
        blocks
            .edges
            .unwrap()
            .into_iter()
            .map(|b| b.unwrap().node.height)
            .collect_vec(),
        rev(0..5).collect_vec()
    );
}

#[tokio::test]
async fn block_connection_last_5() {
    // blocks
    let blocks = (0..10u32)
        .map(|i| FuelBlock {
            fuel_height: i.into(),
            transactions: vec![],
            time: Utc.timestamp(i.into(), 0),
            producer: Default::default(),
        })
        .collect_vec();

    // setup test data in the node
    let mut db = Database::default();
    for block in blocks {
        let id = block.id();
        Storage::<Bytes32, FuelBlock>::insert(&mut db, &id, &block).unwrap();
    }

    // setup server & client
    let srv = run_in_background(configure(db)).await;
    let client = FuelClient::from(srv);

    // run test
    let blocks = client.blocks(None, Some(5), None, None).await.unwrap();
    assert!(blocks.edges.is_some());
    // assert "last" 5 blocks are returned in descending order (latest first)
    assert_eq!(
        blocks
            .edges
            .unwrap()
            .into_iter()
            .map(|b| b.unwrap().node.height)
            .collect_vec(),
        rev(5..10).collect_vec()
    );
}
