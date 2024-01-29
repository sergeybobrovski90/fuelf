use fuel_core::{
    chain_config::{
        CoinConfig,
        Decoder,
        StateConfig,
    },
    database::Database,
    service::{
        Config,
        FuelService,
    },
};
use fuel_core_client::client::{
    pagination::{
        PageDirection,
        PaginationRequest,
    },
    types::primitives::{
        Address,
        AssetId,
        UtxoId,
    },
    FuelClient,
};
use fuel_core_storage::{
    tables::Coins,
    StorageAsMut,
};
use fuel_core_types::{
    entities::coins::coin::Coin,
    fuel_asm::*,
    fuel_tx::TxId,
};
use rstest::rstest;

#[tokio::test]
async fn coin() {
    // setup test data in the node
    let output_index = 5;
    let tx_id = TxId::new([1u8; 32]);
    let coin = CoinConfig {
        output_index: Some(output_index),
        tx_id: Some(tx_id),
        ..Default::default()
    };
    let state = StateConfig {
        coins: vec![coin],
        ..Default::default()
    };
    let config = Config {
        state_reader: Decoder::in_memory(state, 1),
        ..Config::local_node()
    };

    // setup server & client
    let srv = FuelService::from_database(Database::default(), config)
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let utxo_id = UtxoId::new(tx_id, output_index);
    let coin = client.coin(&utxo_id).await.unwrap();
    assert!(coin.is_some());
}

// Backward fails, tracking in https://github.com/FuelLabs/fuel-core/issues/610
#[rstest]
#[tokio::test]
async fn first_5_coins(
    #[values(PageDirection::Forward)] pagination_direction: PageDirection,
) {
    let owner = Address::default();

    // setup test data in the node
    let coins: Vec<_> = (1..10usize)
        .map(|i| Coin {
            utxo_id: UtxoId::new([i as u8; 32].into(), 0),
            owner,
            amount: i as Word,
            asset_id: Default::default(),
            maturity: Default::default(),
            tx_pointer: Default::default(),
        })
        .collect();

    let mut db = Database::default();
    for coin in coins {
        db.storage::<Coins>()
            .insert(&coin.utxo_id.clone(), &coin.compress())
            .unwrap();
    }

    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let coins = client
        .coins(
            &owner,
            None,
            PaginationRequest {
                cursor: None,
                results: 5,
                direction: pagination_direction,
            },
        )
        .await
        .unwrap();
    assert!(!coins.results.is_empty());
    assert_eq!(coins.results.len(), 5)
}

#[tokio::test]
async fn only_asset_id_filtered_coins() {
    let owner = Address::default();
    let asset_id = AssetId::new([1u8; 32]);

    // setup test data in the node
    let coins: Vec<_> = (1..10usize)
        .map(|i| Coin {
            utxo_id: UtxoId::new([i as u8; 32].into(), 0),
            owner,
            amount: i as Word,
            asset_id: if i <= 5 { asset_id } else { Default::default() },
            maturity: Default::default(),
            tx_pointer: Default::default(),
        })
        .collect();

    let mut db = Database::default();
    for coin in coins {
        db.storage::<Coins>()
            .insert(&coin.utxo_id.clone(), &coin.compress())
            .unwrap();
    }

    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let coins = client
        .coins(
            &owner,
            Some(&asset_id),
            PaginationRequest {
                cursor: None,
                results: 10,
                direction: PageDirection::Forward,
            },
        )
        .await
        .unwrap();
    assert!(!coins.results.is_empty());
    assert_eq!(coins.results.len(), 5);
    assert!(coins.results.into_iter().all(|c| asset_id == c.asset_id));
}

#[rstest]
#[tokio::test]
async fn get_coins_forwards_backwards(
    #[values(Address::default(), Address::from([16; 32]))] owner: Address,
    #[values(AssetId::from([1u8; 32]), AssetId::from([32u8; 32]))] asset_id: AssetId,
) {
    // setup test data in the node
    let coins: Vec<Coin> = (1..11usize)
        .map(|i| Coin {
            utxo_id: UtxoId::new([i as u8; 32].into(), 0),
            owner,
            amount: i as Word,
            asset_id,
            maturity: Default::default(),
            tx_pointer: Default::default(),
        })
        .collect();

    let mut db = Database::default();
    for coin in coins {
        db.storage::<Coins>()
            .insert(&coin.utxo_id.clone(), &coin.compress())
            .unwrap();
    }

    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let coins = client
        .coins(
            &owner,
            Some(&asset_id),
            PaginationRequest {
                cursor: None,
                results: 10,
                direction: PageDirection::Forward,
            },
        )
        .await
        .unwrap();
    assert!(!coins.results.is_empty());
    assert_eq!(coins.results.len(), 10);
}
