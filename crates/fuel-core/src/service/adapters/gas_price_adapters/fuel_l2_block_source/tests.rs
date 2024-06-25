#![allow(non_snake_case)]

use super::*;
use fuel_core::database::Database;
use fuel_core_storage::StorageAsMut;
use fuel_core_types::{
    blockchain::block::CompressedBlock,
    fuel_tx::ConsensusParameters,
};
use futures::future::{
    maybe_done,
    MaybeDone,
};

fn l2_source(database: Database) -> FuelL2BlockSource<Database> {
    FuelL2BlockSource {
        frequency: Duration::from_millis(10),
        database,
    }
}

fn l2_source_with_frequency(
    database: Database,
    frequency: Duration,
) -> FuelL2BlockSource<Database> {
    FuelL2BlockSource {
        frequency,
        database,
    }
}

fn params() -> ConsensusParameters {
    ConsensusParameters::default()
}

#[tokio::test]
async fn get_l2_block__gets_expected_value() {
    // given
    let block = CompressedBlock::default();
    let block_height = 1u32.into();
    let params = params();
    let block_gas_limit = params.block_gas_limit();
    let block_info = get_block_info(&block.clone().uncompress(vec![]), block_gas_limit);
    let mut database = Database::default();
    let version = block.header().consensus_parameters_version;
    database
        .storage_as_mut::<ConsensusParametersVersions>()
        .insert(&version, &params)
        .unwrap();
    database
        .storage_as_mut::<FuelBlocks>()
        .insert(&block_height, &block)
        .unwrap();

    let source = l2_source(database);

    // when
    let result = source.get_l2_block(block_height).await.unwrap();

    // then
    assert_eq!(result, block_info);
}

#[tokio::test]
async fn get_l2_block__waits_for_block() {
    // given
    let block_height = 1u32.into();
    let block = CompressedBlock::default();
    let mut database = Database::default();
    let frequency = Duration::from_millis(10);
    let source = l2_source_with_frequency(database.clone(), frequency);
    let params = params();
    let version = block.header().consensus_parameters_version;
    database
        .storage_as_mut::<ConsensusParametersVersions>()
        .insert(&version, &params)
        .unwrap();

    // when
    let mut fut_l2_block = source.get_l2_block(block_height);
    for _ in 0..10 {
        fut_l2_block = match maybe_done(fut_l2_block) {
            MaybeDone::Future(fut) => {
                tokio::time::sleep(frequency).await;
                fut
            }
            _ => panic!("Shouldn't be done yet"),
        };
    }
    database
        .storage_as_mut::<FuelBlocks>()
        .insert(&block_height, &block)
        .unwrap();

    // then
    let actual = fut_l2_block.await.unwrap();
    let uncompressed_block = block.uncompress(vec![]);
    let block_gas_limit = params.block_gas_limit();
    let expected = get_block_info(&uncompressed_block, block_gas_limit);
    assert_eq!(expected, actual);
}

// fn build_block() -> CompressedBlock {
//     todo!()
// }

#[tokio::test]
async fn get_l2_block__calculates_fullness_correctly() {
    // given
    let block = CompressedBlock::default();
    let block_height = 1u32.into();
    let mut database = Database::default();
    let params = params();
    let version = block.header().consensus_parameters_version;
    database
        .storage_as_mut::<ConsensusParametersVersions>()
        .insert(&version, &params)
        .unwrap();
    database
        .storage_as_mut::<FuelBlocks>()
        .insert(&block_height, &block)
        .unwrap();

    let source = l2_source(database);

    // when
    let result = source.get_l2_block(block_height).await.unwrap();

    // then
    let actual = result.fullness;
    let block_gas_limit = params.block_gas_limit();
    let used = 1_000;
    let expected = (used, block_gas_limit);
    assert_eq!(expected, actual);
}

#[tokio::test]
async fn get_l2_block__calculates_block_bytes_correctly() {}

#[tokio::test]
async fn get_l2_block__retrieves_gas_price_correctly() {}
