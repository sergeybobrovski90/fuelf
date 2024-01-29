use crate::{
    database::{
        Column,
        Database,
    },
    service::config::Config,
};
use anyhow::anyhow;
use fuel_core_chain_config::{
    CoinConfig,
    ContractConfig,
    GenesisCommitment,
    MessageConfig,
};

use fuel_core_importer::Importer;
use fuel_core_storage::{
    tables::{
        Coins,
        ContractsInfo,
        ContractsLatestUtxo,
        ContractsRawCode,
        FuelBlocks,
        Messages,
    },
    transactional::Transactional,
    MerkleRoot,
    StorageAsMut,
};
use fuel_core_types::{
    blockchain::{
        block::Block,
        consensus::{
            Consensus,
            Genesis,
        },
        header::{
            ApplicationHeader,
            ConsensusHeader,
            PartialBlockHeader,
        },
        primitives::Empty,
        SealedBlock,
    },
    entities::{
        coins::coin::CompressedCoin,
        contract::ContractUtxoInfo,
        message::Message,
    },
    fuel_tx::{
        Contract,
        UtxoId,
    },
    fuel_types::{
        bytes::WORD_SIZE,
        BlockHeight,
        Bytes32,
    },
    services::block_importer::{
        ImportResult,
        UncommittedResult as UncommittedImportResult,
    },
};
use itertools::Itertools;

mod runner;
pub use runner::{
    GenesisRunner,
    TransactionOpener,
};

mod workers;
use self::workers::GenesisWorkers;

/// Loads state from the chain config into database
pub async fn maybe_initialize_state(
    config: &Config,
    database: &Database,
) -> anyhow::Result<()> {
    // check if chain is initialized
    if database.ids_of_latest_block()?.is_none() {
        let workers = GenesisWorkers::new(
            database.clone(),
            config.chain_config.height.unwrap_or_default(),
            config.state_reader.clone(),
        );

        import_chain_state(workers).await?;
        commit_genesis_block(config, database)?;
        cleanup_genesis_progress(database)?;
    }

    Ok(())
}

async fn import_chain_state(workers: GenesisWorkers) -> anyhow::Result<()> {
    if let Err(e) = workers.run_imports().await {
        workers.shutdown();
        tokio::select! {
            _ = workers.finished() => {}
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
                return Err(anyhow!("Timeout while importing genesis state"));
            }
        };

        return Err(e)
    }

    workers.compute_contracts_root().await?;

    Ok(())
}

fn commit_genesis_block(
    config: &Config,
    original_database: &Database,
) -> anyhow::Result<()> {
    let mut database_transaction = Transactional::transaction(original_database);
    let database = database_transaction.as_mut();

    let genesis = Genesis {
        chain_config_hash: config.chain_config.root()?.into(),
        coins_root: original_database.genesis_coin_root()?.into(),
        messages_root: original_database.genesis_messages_root()?.into(),
        contracts_root: original_database.genesis_contracts_root()?.into(),
    };

    let block = Block::new(
        PartialBlockHeader {
            application: ApplicationHeader::<Empty> {
                // TODO: Set `da_height` based on the chain config.
                da_height: Default::default(),
                generated: Empty,
            },
            consensus: ConsensusHeader::<Empty> {
                // The genesis is a first block, so previous root is zero.
                prev_root: Bytes32::zeroed(),
                // The initial height is defined by the `ChainConfig`.
                height: config.chain_config.height.unwrap_or_default(),
                time: fuel_core_types::tai64::Tai64::UNIX_EPOCH,
                generated: Empty,
            },
        },
        // Genesis block doesn't have any transaction.
        vec![],
        &[],
    );

    let block_id = block.id();
    database.storage::<FuelBlocks>().insert(
        &block_id,
        &block.compress(&config.chain_config.consensus_parameters.chain_id),
    )?;
    let consensus = Consensus::Genesis(genesis);
    let block = SealedBlock {
        entity: block,
        consensus,
    };

    let importer = Importer::new(
        config.block_importer.clone(),
        original_database.clone(),
        (),
        (),
    );
    importer.commit_result(UncommittedImportResult::new(
        ImportResult::new_from_local(block, vec![]),
        database_transaction,
    ))?;

    Ok(())
}

fn cleanup_genesis_progress(database: &Database) -> anyhow::Result<()> {
    let mut database_transaction = Transactional::transaction(database);
    let database = database_transaction.as_mut();

    database.delete_all(Column::GenesisMetadata)?;
    database.delete_all(Column::GenesisCoinRoots)?;
    database.delete_all(Column::GenesisMessageRoots)?;
    database.delete_all(Column::GenesisContractBalanceRoots)?;
    database.delete_all(Column::GenesisContractStateRoots)?;
    database.delete_all(Column::GenesisContractRoots)?;
    database.delete_all(Column::GenesisContractIds)?;

    database_transaction.commit()?;

    Ok(())
}

fn generated_utxo_id(output_index: u64) -> UtxoId {
    UtxoId::new(
        // generated transaction id([0..[out_index/255]])
        Bytes32::try_from(
            (0..(Bytes32::LEN - WORD_SIZE))
                .map(|_| 0u8)
                .chain((output_index / 255).to_be_bytes())
                .collect_vec()
                .as_slice(),
        )
        .expect("Incorrect genesis transaction id byte length"),
        (output_index % 255) as u8,
    )
}

fn init_coin(
    db: &mut Database,
    coin: &CoinConfig,
    output_index: u64,
    height: BlockHeight,
) -> anyhow::Result<MerkleRoot> {
    // TODO: Store merkle sum tree root over coins with unspecified utxo ids.
    let utxo_id = coin.utxo_id().unwrap_or(generated_utxo_id(output_index));

    let coin = CompressedCoin {
        owner: coin.owner,
        amount: coin.amount,
        asset_id: coin.asset_id,
        maturity: coin.maturity.unwrap_or_default(),
        tx_pointer: coin.tx_pointer(),
    };

    // ensure coin can't point to blocks in the future
    let coin_height = coin.tx_pointer.block_height();
    if coin_height > height {
        return Err(anyhow!(
            "coin tx_pointer height ({coin_height}) cannot be greater than genesis block ({height})"
        ))
    }

    if db.storage::<Coins>().insert(&utxo_id, &coin)?.is_some() {
        return Err(anyhow!("Coin should not exist"));
    }
    coin.root()
}

fn init_contract(
    db: &mut Database,
    contract_config: &ContractConfig,
    output_index: u64,
    height: BlockHeight,
) -> anyhow::Result<()> {
    let contract = Contract::from(contract_config.code.as_slice());
    let salt = contract_config.salt;
    let root = contract.root();
    let contract_id = contract_config.contract_id;
    #[allow(clippy::cast_possible_truncation)]
    let utxo_id = contract_config
        .utxo_id()
        .unwrap_or(generated_utxo_id(output_index));

    let tx_pointer = contract_config.tx_pointer();
    if tx_pointer.block_height() > height {
        return Err(anyhow!(
            "contract tx_pointer cannot be greater than genesis block"
        ));
    }

    // insert contract code
    if db
        .storage::<ContractsRawCode>()
        .insert(&contract_id, contract.as_ref())?
        .is_some()
    {
        return Err(anyhow!("Contract code should not exist"));
    }

    // insert contract root
    if db
        .storage::<ContractsInfo>()
        .insert(&contract_id, &(salt, root))?
        .is_some()
    {
        return Err(anyhow!("Contract info should not exist"));
    }
    if db
        .storage::<ContractsLatestUtxo>()
        .insert(
            &contract_id,
            &ContractUtxoInfo {
                utxo_id,
                tx_pointer,
            },
        )?
        .is_some()
    {
        return Err(anyhow!("Contract utxo should not exist"));
    }
    Ok(())
}

fn init_da_message(db: &mut Database, msg: &MessageConfig) -> anyhow::Result<MerkleRoot> {
    let message = Message {
        sender: msg.sender,
        recipient: msg.recipient,
        nonce: msg.nonce,
        amount: msg.amount,
        data: msg.data.clone(),
        da_height: msg.da_height,
    };

    if db
        .storage::<Messages>()
        .insert(message.id(), &message)?
        .is_some()
    {
        return Err(anyhow!("Message should not exist"));
    }

    message.root()
}

#[cfg(test)]
mod tests {
    use crate::{
        database::{
            genesis_progress::GenesisResource,
            Column,
            Database,
        },
        service::{
            config::Config,
            genesis::{
                generated_utxo_id,
                maybe_initialize_state,
            },
            FuelService,
        },
    };
    use fuel_core_chain_config::{
        ChainConfig,
        CoinConfig,
        ContractBalanceConfig,
        ContractConfig,
        ContractStateConfig,
        Decoder,
        MessageConfig,
        StateConfig,
        MAX_GROUP_SIZE,
    };
    use fuel_core_storage::{
        tables::{
            Coins,
            ContractsAssets,
            ContractsState,
            Messages,
        },
        StorageAsRef,
    };
    use fuel_core_types::{
        blockchain::primitives::DaBlockHeight,
        entities::{
            coins::coin::Coin,
            message::Message,
        },
        fuel_asm::op,
        fuel_tx::UtxoId,
        fuel_types::{
            Address,
            AssetId,
            BlockHeight,
            ContractId,
            Salt,
        },
        fuel_vm::Contract,
    };
    use itertools::Itertools;
    use rand::{
        rngs::StdRng,
        Rng,
        RngCore,
        SeedableRng,
    };

    #[tokio::test]
    async fn config_initializes_chain_name() {
        let test_name = "test_net_123".to_string();
        let service_config = Config {
            chain_config: ChainConfig {
                chain_name: test_name.clone(),
                ..ChainConfig::local_testnet()
            },
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        assert_eq!(
            test_name,
            db.get_chain_name()
                .unwrap()
                .expect("Expected a chain name to be set")
        )
    }

    #[tokio::test]
    async fn config_initializes_block_height() {
        let height = BlockHeight::from(99u32);
        let service_config = Config {
            chain_config: ChainConfig {
                height: Some(height),
                ..ChainConfig::local_testnet()
            },
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        assert_eq!(
            height,
            db.latest_height()
                .expect("Expected a block height to be set")
        )
    }

    #[tokio::test]
    async fn genesis_columns_are_cleared_after_import() {
        let mut rng = StdRng::seed_from_u64(10);

        let coins = (0..1000)
            .map(|_| CoinConfig {
                tx_id: None,
                output_index: None,
                tx_pointer_block_height: None,
                tx_pointer_tx_idx: None,
                maturity: None,
                owner: Default::default(),
                amount: 10,
                asset_id: Default::default(),
            })
            .collect_vec();

        let messages = (0..1000)
            .map(|_| MessageConfig {
                sender: rng.gen(),
                recipient: rng.gen(),
                nonce: rng.gen(),
                amount: rng.gen(),
                data: vec![rng.gen()],
                da_height: DaBlockHeight(0),
            })
            .collect_vec();

        let contracts = (0..1000)
            .map(|_| given_contract_config(&mut rng))
            .collect_vec();

        let contract_state = (0..1000)
            .map(|_| ContractStateConfig {
                contract_id: rng.gen(),
                key: rng.gen(),
                value: rng.gen(),
            })
            .collect_vec();

        let contract_balance = (0..1000)
            .map(|_| ContractBalanceConfig {
                contract_id: rng.gen(),
                asset_id: rng.gen(),
                amount: rng.next_u64(),
            })
            .collect_vec();

        let state = StateConfig {
            coins,
            messages,
            contracts,
            contract_state,
            contract_balance,
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let service_config = Config {
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        use strum::IntoEnumIterator;
        for key in GenesisResource::iter() {
            assert!(db.genesis_progress(&key).is_none());
        }
        assert!(db
            .genesis_roots(Column::GenesisCoinRoots)
            .unwrap()
            .next()
            .is_none());
        assert!(db
            .genesis_roots(Column::GenesisMessageRoots)
            .unwrap()
            .next()
            .is_none());
        assert!(db
            .genesis_roots(Column::GenesisContractRoots)
            .unwrap()
            .next()
            .is_none());
        assert!(db
            .genesis_roots(Column::GenesisContractStateRoots)
            .unwrap()
            .next()
            .is_none());
        assert!(db
            .genesis_roots(Column::GenesisContractBalanceRoots)
            .unwrap()
            .next()
            .is_none());
        assert!(db.genesis_contract_ids_iter().next().is_none());
    }

    #[tokio::test]
    async fn config_state_initializes_multiple_coins_with_different_owners_and_asset_ids()
    {
        let mut rng = StdRng::seed_from_u64(10);

        // a coin with all options set
        let alice: Address = rng.gen();
        let asset_id_alice: AssetId = rng.gen();
        let alice_value = rng.gen();
        let alice_maturity: BlockHeight = rng.next_u32().into();
        let alice_block_created: BlockHeight = rng.next_u32().into();
        let alice_block_created_tx_idx = rng.gen();
        let alice_tx_id = rng.gen();
        let alice_output_index = rng.gen();
        let alice_utxo_id = UtxoId::new(alice_tx_id, alice_output_index);

        // a coin with minimal options set
        let bob: Address = rng.gen();
        let asset_id_bob: AssetId = rng.gen();
        let bob_value = rng.gen();

        let state = StateConfig {
            coins: vec![
                CoinConfig {
                    tx_id: Some(alice_tx_id),
                    output_index: Some(alice_output_index),
                    tx_pointer_block_height: Some(alice_block_created),
                    tx_pointer_tx_idx: Some(alice_block_created_tx_idx),
                    maturity: Some(alice_maturity),
                    owner: alice,
                    amount: alice_value,
                    asset_id: asset_id_alice,
                },
                CoinConfig {
                    tx_id: None,
                    output_index: None,
                    tx_pointer_block_height: None,
                    tx_pointer_tx_idx: None,
                    maturity: None,
                    owner: bob,
                    amount: bob_value,
                    asset_id: asset_id_bob,
                },
            ],
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let starting_height = {
            let mut h: u32 = alice_block_created.into();
            h = h.saturating_add(rng.next_u32());
            Some(h.into())
        };
        let service_config = Config {
            chain_config: ChainConfig {
                height: starting_height,
                ..ChainConfig::local_testnet()
            },
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        let alice_coins = get_coins(&db, &alice);
        let bob_coins = get_coins(&db, &bob);

        assert!(matches!(
            alice_coins.as_slice(),
            &[Coin {
                utxo_id,
                owner,
                amount,
                asset_id,
                tx_pointer,
                maturity,
                ..
            }] if utxo_id == alice_utxo_id
            && owner == alice
            && amount == alice_value
            && asset_id == asset_id_alice
            && tx_pointer.block_height() == alice_block_created
            && maturity == alice_maturity,
        ));
        assert!(matches!(
            bob_coins.as_slice(),
            &[Coin {
                owner,
                amount,
                asset_id,
                ..
            }] if owner == bob
            && amount == bob_value
            && asset_id == asset_id_bob
        ));
    }

    #[tokio::test]
    async fn config_state_initializes_contract_state() {
        let mut rng = StdRng::seed_from_u64(10);

        let salt: Salt = rng.gen();
        let contract = Contract::from(op::ret(0x10).to_bytes().to_vec());
        let root = contract.root();
        let contract_id = contract.id(&salt, &root, &Contract::default_state_root());

        let test_key = rng.gen();
        let test_value = rng.gen();
        let contract_state = ContractStateConfig {
            contract_id,
            key: test_key,
            value: test_value,
        };

        let state = StateConfig {
            contracts: vec![ContractConfig {
                contract_id,
                code: contract.into(),
                salt,
                tx_id: Some(rng.gen()),
                output_index: Some(rng.gen()),
                tx_pointer_block_height: Some(0u32.into()),
                tx_pointer_tx_idx: Some(rng.gen()),
            }],
            contract_state: vec![contract_state],
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let service_config = Config {
            chain_config: ChainConfig::local_testnet(),
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        let ret = db
            .storage::<ContractsState>()
            .get(&(&contract_id, &test_key).into())
            .unwrap()
            .expect("Expect a state entry to exist with test_key")
            .into_owned();

        assert_eq!(test_value, ret)
    }

    #[tokio::test]
    async fn tests_init_da_msgs() {
        let mut rng = StdRng::seed_from_u64(32492);

        let msg = MessageConfig {
            sender: rng.gen(),
            recipient: rng.gen(),
            nonce: rng.gen(),
            amount: rng.gen(),
            data: vec![rng.gen()],
            da_height: DaBlockHeight(0),
        };

        let state = StateConfig {
            messages: vec![msg.clone()],
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let config = Config {
            state_reader,
            ..Config::local_node()
        };

        let db = &Database::default();

        maybe_initialize_state(&config, db).await.unwrap();

        let expected_msg: Message = msg.into();

        let ret_msg = db
            .storage::<Messages>()
            .get(expected_msg.id())
            .unwrap()
            .unwrap()
            .into_owned();

        assert_eq!(expected_msg, ret_msg);
    }

    #[tokio::test]
    async fn config_state_initializes_contract_balance() {
        let mut rng = StdRng::seed_from_u64(10);

        let salt: Salt = rng.gen();
        let contract = Contract::from(op::ret(0x10).to_bytes().to_vec());
        let root = contract.root();
        let contract_id = contract.id(&salt, &root, &Contract::default_state_root());

        let test_asset_id = rng.gen();
        let test_balance = rng.next_u64();
        let contract_balance = ContractBalanceConfig {
            contract_id,
            asset_id: test_asset_id,
            amount: test_balance,
        };

        let state = StateConfig {
            contracts: vec![ContractConfig {
                contract_id,
                code: contract.into(),
                salt,
                tx_id: None,
                output_index: None,
                tx_pointer_block_height: None,
                tx_pointer_tx_idx: None,
            }],
            contract_balance: vec![contract_balance],
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let service_config = Config {
            chain_config: ChainConfig::local_testnet(),
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        let ret = db
            .storage::<ContractsAssets>()
            .get(&(&contract_id, &test_asset_id).into())
            .unwrap()
            .expect("Expected a balance to be present")
            .into_owned();

        assert_eq!(test_balance, ret)
    }

    #[tokio::test]
    async fn coin_tx_pointer_cant_exceed_genesis_height() {
        let state = StateConfig {
            coins: vec![CoinConfig {
                tx_id: None,
                output_index: None,
                // set txpointer height > genesis height
                tx_pointer_block_height: Some(BlockHeight::from(11u32)),
                tx_pointer_tx_idx: Some(0),
                maturity: None,
                owner: Default::default(),
                amount: 10,
                asset_id: Default::default(),
            }],
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let service_config = Config {
            chain_config: ChainConfig {
                height: Some(BlockHeight::from(10u32)),
                ..ChainConfig::local_testnet()
            },
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        let init_result = FuelService::from_database(db.clone(), service_config).await;

        assert!(init_result.is_err())
    }

    #[tokio::test]
    async fn contract_tx_pointer_cant_exceed_genesis_height() {
        let mut rng = StdRng::seed_from_u64(10);

        let test_asset_id: AssetId = rng.gen();
        let test_balance: u64 = rng.next_u64();
        let contract_id = Default::default();
        let balances = vec![ContractBalanceConfig {
            contract_id,
            asset_id: test_asset_id,
            amount: test_balance,
        }];
        let salt: Salt = rng.gen();
        let contract = Contract::from(op::ret(0x10).to_bytes().to_vec());

        let state = StateConfig {
            contracts: vec![ContractConfig {
                contract_id: ContractId::from(*contract_id),
                code: contract.into(),
                salt,
                tx_id: None,
                output_index: None,
                // set txpointer height > genesis height
                tx_pointer_block_height: Some(BlockHeight::from(11u32)),
                tx_pointer_tx_idx: Some(0),
            }],
            contract_balance: balances,
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let service_config = Config {
            chain_config: ChainConfig {
                height: Some(BlockHeight::from(10u32)),
                ..ChainConfig::local_testnet()
            },
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        let init_result = FuelService::from_database(db.clone(), service_config).await;

        assert!(init_result.is_err())
    }

    fn get_coins(db: &Database, owner: &Address) -> Vec<Coin> {
        db.owned_coins_ids(owner, None, None)
            .map(|r| {
                let coin_id = r.unwrap();
                db.storage::<Coins>()
                    .get(&coin_id)
                    .map(|v| v.unwrap().into_owned().uncompress(coin_id))
                    .unwrap()
            })
            .collect()
    }

    fn given_contract_config(rng: &mut StdRng) -> ContractConfig {
        let contract = Contract::from(op::ret(0x10).to_bytes().to_vec());
        let salt = rng.gen();
        let root = contract.root();
        let contract_id = contract.id(&salt, &root, &Contract::default_state_root());

        ContractConfig {
            contract_id,
            code: contract.into(),
            salt,
            tx_id: None,
            output_index: None,
            tx_pointer_block_height: None,
            tx_pointer_tx_idx: None,
        }
    }

    fn given_contract_config_with_tx(rng: &mut StdRng) -> ContractConfig {
        let mut config = given_contract_config(rng);
        config.tx_id = Some(rng.gen());
        config.output_index = Some(rng.gen());
        config.tx_pointer_block_height = Some(0.into());
        config.tx_pointer_tx_idx = Some(rng.gen());

        config
    }

    #[tokio::test]
    async fn config_state_initializes_contract() {
        let mut rng = StdRng::seed_from_u64(10);
        let config = given_contract_config_with_tx(&mut rng);
        let contract_id = config.contract_id;

        let state = StateConfig {
            contracts: vec![config.clone()],
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let service_config = Config {
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        let initialized_contract = db.get_contract_config(contract_id).unwrap();

        assert_eq!(initialized_contract, config);
    }

    #[tokio::test]
    async fn initializes_contracts_with_generated_tx_and_output_idx() {
        let mut rng = StdRng::seed_from_u64(10);

        let contracts = (0..1000)
            .map(|_| given_contract_config(&mut rng))
            .collect_vec();

        let state = StateConfig {
            contracts: contracts.clone(),
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let service_config = Config {
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        for (idx, contract) in contracts.iter().enumerate() {
            let initialized_contract =
                db.get_contract_config(contract.contract_id).unwrap();

            let expected_utxo_id = generated_utxo_id(idx as u64);

            assert_eq!(
                initialized_contract.tx_id.unwrap(),
                *expected_utxo_id.tx_id()
            );
            assert_eq!(
                initialized_contract.output_index.unwrap(),
                expected_utxo_id.output_index()
            );
        }
    }

    #[tokio::test]
    async fn initializes_coins_with_generated_tx_and_output_idx() {
        let coins = (0..1000)
            .map(|_| CoinConfig {
                tx_id: None,
                output_index: None,
                tx_pointer_block_height: None,
                tx_pointer_tx_idx: None,
                maturity: None,
                owner: Default::default(),
                amount: 10,
                asset_id: Default::default(),
            })
            .collect_vec();

        let state = StateConfig {
            coins: coins.clone(),
            ..Default::default()
        };
        let state_reader = Decoder::in_memory(state, MAX_GROUP_SIZE);

        let service_config = Config {
            state_reader,
            ..Config::local_node()
        };

        let db = Database::default();
        FuelService::from_database(db.clone(), service_config)
            .await
            .unwrap();

        for (idx, _) in coins.iter().enumerate() {
            let expected_utxo_id = generated_utxo_id(idx as u64);
            db.coin(&expected_utxo_id)
                .expect("Coin with expected utxo id should exist");
        }
    }
}
