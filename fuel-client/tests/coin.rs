use fuel_core::{
    chain_config::{CoinConfig, StateConfig},
    database::Database,
    model::coin::{Coin, CoinStatus},
    service::{Config, FuelService},
};
use fuel_gql_client::client::{
    schema::coin::CoinStatus as SchemaCoinStatus, FuelClient, PageDirection, PaginationRequest,
};
use fuel_storage::Storage;
use fuel_tx::{AssetId, UtxoId};
use fuel_vm::prelude::{Address, Bytes32, Word};

#[tokio::test]
async fn coin() {
    // setup test data in the node
    let coin = Coin {
        owner: Default::default(),
        amount: 0,
        color: Default::default(),
        maturity: Default::default(),
        status: CoinStatus::Unspent,
        block_created: Default::default(),
    };

    let utxo_id = UtxoId::new(Default::default(), 5);

    let mut db = Database::default();
    Storage::<UtxoId, Coin>::insert(&mut db, &utxo_id, &coin).unwrap();
    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let coin = client
        .coin(format!("{:#x}", utxo_id).as_str())
        .await
        .unwrap();
    assert!(coin.is_some());
}

#[tokio::test]
async fn first_5_coins() {
    let owner = Address::default();

    // setup test data in the node
    let coins: Vec<(UtxoId, Coin)> = (1..10usize)
        .map(|i| {
            let coin = Coin {
                owner,
                amount: i as Word,
                color: Default::default(),
                maturity: Default::default(),
                status: CoinStatus::Unspent,
                block_created: Default::default(),
            };

            let utxo_id = UtxoId::new(Bytes32::from([i as u8; 32]), 0);
            (utxo_id, coin)
        })
        .collect();

    let mut db = Database::default();
    for (utxo_id, coin) in coins {
        Storage::<UtxoId, Coin>::insert(&mut db, &utxo_id, &coin).unwrap();
    }

    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let coins = client
        .coins(
            format!("{:#x}", owner).as_str(),
            None,
            PaginationRequest {
                cursor: None,
                results: 5,
                direction: PageDirection::Forward,
            },
        )
        .await
        .unwrap();
    assert!(!coins.results.is_empty());
    assert_eq!(coins.results.len(), 5)
}

#[tokio::test]
async fn only_color_filtered_coins() {
    let owner = Address::default();
    let color = AssetId::new([1u8; 32]);

    // setup test data in the node
    let coins: Vec<(UtxoId, Coin)> = (1..10usize)
        .map(|i| {
            let coin = Coin {
                owner,
                amount: i as Word,
                color: if i <= 5 { color } else { Default::default() },
                maturity: Default::default(),
                status: CoinStatus::Unspent,
                block_created: Default::default(),
            };

            let utxo_id = UtxoId::new(Bytes32::from([i as u8; 32]), 0);
            (utxo_id, coin)
        })
        .collect();

    let mut db = Database::default();
    for (id, coin) in coins {
        Storage::<UtxoId, Coin>::insert(&mut db, &id, &coin).unwrap();
    }

    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let coins = client
        .coins(
            format!("{:#x}", owner).as_str(),
            Some(format!("{:#x}", AssetId::new([1u8; 32])).as_str()),
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
    assert!(coins.results.into_iter().all(|c| color == c.color.into()));
}

#[tokio::test]
async fn only_unspent_coins() {
    let owner = Address::default();

    // setup test data in the node
    let coins: Vec<(UtxoId, Coin)> = (1..10usize)
        .map(|i| {
            let coin = Coin {
                owner,
                amount: i as Word,
                color: Default::default(),
                maturity: Default::default(),
                status: if i <= 5 {
                    CoinStatus::Unspent
                } else {
                    CoinStatus::Spent
                },
                block_created: Default::default(),
            };

            let utxo_id = UtxoId::new(Bytes32::from([i as u8; 32]), 0);
            (utxo_id, coin)
        })
        .collect();

    let mut db = Database::default();
    for (id, coin) in coins {
        Storage::<UtxoId, Coin>::insert(&mut db, &id, &coin).unwrap();
    }

    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let coins = client
        .coins(
            format!("{:#x}", owner).as_str(),
            None,
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
    assert!(coins
        .results
        .into_iter()
        .all(|c| c.status == SchemaCoinStatus::Unspent));
}

#[tokio::test]
async fn coins_to_spend() {
    let owner = Address::default();
    let color_a = AssetId::new([1u8; 32]);
    let color_b = AssetId::new([2u8; 32]);

    // setup config
    let mut config = Config::local_node();
    config.chain_conf.initial_state = Some(StateConfig {
        height: None,
        contracts: None,
        coins: Some(
            vec![
                (owner, 50, color_a),
                (owner, 100, color_a),
                (owner, 150, color_a),
                (owner, 50, color_b),
                (owner, 100, color_b),
                (owner, 150, color_b),
            ]
            .into_iter()
            .map(|(owner, amount, color)| CoinConfig {
                tx_id: None,
                output_index: None,
                block_created: None,
                maturity: None,
                owner,
                amount,
                color,
            })
            .collect(),
        ),
    });

    // setup server & client
    let srv = FuelService::new_node(config).await.unwrap();
    let client = FuelClient::from(srv.bound_address);

    // empty spend_query
    let coins = client
        .coins_to_spend(format!("{:#x}", owner).as_str(), vec![], None)
        .await
        .unwrap();
    assert!(coins.is_empty());

    // spend_query for 1 a and 1 b
    let coins = client
        .coins_to_spend(
            format!("{:#x}", owner).as_str(),
            vec![
                (format!("{:#x}", color_a).as_str(), 1),
                (format!("{:#x}", color_b).as_str(), 1),
            ],
            None,
        )
        .await
        .unwrap();
    assert_eq!(coins.len(), 2);

    // spend_query for 300 a and 300 b
    let coins = client
        .coins_to_spend(
            format!("{:#x}", owner).as_str(),
            vec![
                (format!("{:#x}", color_a).as_str(), 300),
                (format!("{:#x}", color_b).as_str(), 300),
            ],
            None,
        )
        .await
        .unwrap();
    assert_eq!(coins.len(), 6);

    // not enough coins
    let coins = client
        .coins_to_spend(
            format!("{:#x}", owner).as_str(),
            vec![
                (format!("{:#x}", color_a).as_str(), 301),
                (format!("{:#x}", color_b).as_str(), 301),
            ],
            None,
        )
        .await;
    assert!(coins.is_err());

    // not enough inputs
    let coins = client
        .coins_to_spend(
            format!("{:#x}", owner).as_str(),
            vec![
                (format!("{:#x}", color_a).as_str(), 300),
                (format!("{:#x}", color_b).as_str(), 300),
            ],
            5.into(),
        )
        .await;
    assert!(coins.is_err());
}
