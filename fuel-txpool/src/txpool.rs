use crate::{
    containers::{
        dependency::Dependency,
        price_sort::PriceSort,
    },
    types::*,
    Config,
    Error,
};
use anyhow::anyhow;
use fuel_core_interfaces::{
    common::fuel_tx::{
        Chargeable,
        CheckedTransaction,
        Fully,
        IntoChecked,
        Partially,
        Transaction,
        UniqueIdentifier,
    },
    model::{
        ArcPoolTx,
        FuelBlock,
        TxInfo,
    },
    txpool::{
        Either,
        InsertionResult,
        TxPoolDb,
        TxStatus,
        TxStatusBroadcast,
    },
};
use std::{
    cmp::Reverse,
    collections::HashMap,
    ops::Deref,
    sync::Arc,
};
use tokio::sync::{
    broadcast,
    RwLock,
};

#[derive(Debug, Clone)]
pub struct TxPool {
    by_hash: HashMap<TxId, TxInfo>,
    by_gas_price: PriceSort,
    by_dependency: Dependency,
    config: Config,
}

impl TxPool {
    pub fn new(config: Config) -> Self {
        let max_depth = config.max_depth;
        Self {
            by_hash: HashMap::new(),
            by_gas_price: PriceSort::default(),
            by_dependency: Dependency::new(max_depth, config.utxo_validation),
            config,
        }
    }
    pub fn txs(&self) -> &HashMap<TxId, TxInfo> {
        &self.by_hash
    }

    pub fn dependency(&self) -> &Dependency {
        &self.by_dependency
    }

    // this is atomic operation. Return removed(pushed out/replaced) transactions
    pub async fn insert_inner(
        &mut self,
        tx: Arc<Transaction>,
        db: &dyn TxPoolDb,
    ) -> anyhow::Result<InsertionResult> {
        let current_height = db.current_block_height()?;

        // verify gas price is at least the minimum
        self.verify_tx_min_gas_price(&tx)?;

        let tx = if self.config.utxo_validation {
            let tx: CheckedTransaction<Fully> = tx
                .deref()
                .clone()
                .into_checked(
                    current_height.into(),
                    &self.config.chain_config.transaction_parameters,
                )?
                .into();

            Arc::new(match tx {
                CheckedTransaction::Script(script) => {
                    PoolTransaction::Script(Either::Fully(script))
                }
                CheckedTransaction::Create(create) => {
                    PoolTransaction::Create(Either::Fully(create))
                }
            })
        } else {
            let tx: CheckedTransaction<Partially> = tx
                .deref()
                .clone()
                .into_checked_partially(
                    current_height.into(),
                    &self.config.chain_config.transaction_parameters,
                )?
                .into();

            Arc::new(match tx {
                CheckedTransaction::Script(script) => {
                    PoolTransaction::Script(Either::Partially(script))
                }
                CheckedTransaction::Create(create) => {
                    PoolTransaction::Create(Either::Partially(create))
                }
            })
        };

        if !tx.is_computed() {
            return Err(Error::NoMetadata.into())
        }

        // verify max gas is less than block limit
        if tx.max_gas() > self.config.chain_config.block_gas_limit {
            return Err(Error::NotInsertedMaxGasLimit {
                tx_gas: tx.max_gas(),
                block_limit: self.config.chain_config.block_gas_limit,
            }
            .into())
        }

        // verify predicates
        if !tx.check_predicates(self.config.chain_config.transaction_parameters) {
            return Err(anyhow!("transaction predicate verification failed"))
        }

        if self.by_hash.contains_key(&tx.id()) {
            return Err(Error::NotInsertedTxKnown.into())
        }

        let mut max_limit_hit = false;
        // check if we are hitting limit of pool
        if self.by_hash.len() >= self.config.max_tx {
            max_limit_hit = true;
            // limit is hit, check if we can push out lowest priced tx
            let lowest_price = self.by_gas_price.lowest_price();
            if lowest_price >= tx.price() {
                return Err(Error::NotInsertedLimitHit.into())
            }
        }
        // check and insert dependency
        let rem = self.by_dependency.insert(&self.by_hash, db, &tx).await?;
        self.by_hash.insert(tx.id(), TxInfo::new(tx.clone()));
        self.by_gas_price.insert(&tx);

        // if some transaction were removed so we don't need to check limit
        let removed = if rem.is_empty() {
            if max_limit_hit {
                // remove last tx from sort
                let rem_tx = self.by_gas_price.last().unwrap(); // safe to unwrap limit is hit
                self.remove_inner(&rem_tx);
                vec![rem_tx]
            } else {
                Vec::new()
            }
        } else {
            // remove ret from by_hash and from by_price
            for rem in rem.iter() {
                self.by_hash
                    .remove(&rem.id())
                    .expect("Expect to hash of tx to be present");
                self.by_gas_price.remove(rem);
            }

            rem
        };

        Ok(InsertionResult {
            inserted: tx,
            removed,
        })
    }

    /// Return all sorted transactions that are includable in next block.
    pub fn sorted_includable(&self) -> Vec<ArcPoolTx> {
        self.by_gas_price
            .sort
            .iter()
            .rev()
            .map(|(_, tx)| tx.clone())
            .collect()
    }

    pub fn remove_inner(&mut self, tx: &ArcPoolTx) -> Vec<ArcPoolTx> {
        self.remove_by_tx_id(&tx.id())
    }

    /// remove transaction from pool needed on user demand. Low priority
    pub fn remove_by_tx_id(&mut self, tx_id: &TxId) -> Vec<ArcPoolTx> {
        if let Some(tx) = self.by_hash.remove(tx_id) {
            let removed = self
                .by_dependency
                .recursively_remove_all_dependencies(&self.by_hash, tx.tx().clone());
            for remove in removed.iter() {
                self.by_gas_price.remove(remove);
                self.by_hash.remove(&remove.id());
            }
            return removed
        }
        Vec::new()
    }

    fn verify_tx_min_gas_price(&mut self, tx: &Transaction) -> Result<(), Error> {
        let price = match tx {
            Transaction::Script(script) => script.price(),
            Transaction::Create(create) => create.price(),
        };
        if price < self.config.min_gas_price {
            return Err(Error::NotInsertedGasPriceTooLow)
        }
        Ok(())
    }

    /// Import a set of transactions from network gossip or GraphQL endpoints.
    pub async fn insert(
        txpool: &RwLock<Self>,
        db: &dyn TxPoolDb,
        tx_status_sender: broadcast::Sender<TxStatusBroadcast>,
        txs: &Vec<Arc<Transaction>>,
    ) -> Vec<anyhow::Result<InsertionResult>> {
        // Check if that data is okay (witness match input/output, and if recovered signatures ara valid).
        // should be done before transaction comes to txpool, or before it enters RwLocked region.
        let mut res = Vec::new();
        for tx in txs.iter() {
            let mut pool = txpool.write().await;
            res.push(pool.insert_inner(tx.clone(), db).await)
        }
        // announce to subscribers
        for ret in res.iter() {
            match ret {
                Ok(InsertionResult { removed, inserted }) => {
                    for removed in removed {
                        // small todo there is possibility to have removal reason (ReplacedByHigherGas, DependencyRemoved)
                        // but for now it is okay to just use Error::Removed.
                        let _ = tx_status_sender.send(TxStatusBroadcast {
                            tx: removed.clone(),
                            status: TxStatus::SqueezedOut {
                                reason: Error::Removed,
                            },
                        });
                    }
                    let _ = tx_status_sender.send(TxStatusBroadcast {
                        tx: inserted.clone(),
                        status: TxStatus::Submitted,
                    });
                }
                Err(_) => {
                    // @dev should not broadcast tx if error occurred
                }
            }
        }
        res
    }

    /// find all tx by its hash
    pub async fn find(txpool: &RwLock<Self>, hashes: &[TxId]) -> Vec<Option<TxInfo>> {
        let mut res = Vec::with_capacity(hashes.len());
        let pool = txpool.read().await;
        for hash in hashes {
            res.push(pool.txs().get(hash).cloned());
        }
        res
    }

    pub async fn find_one(txpool: &RwLock<Self>, hash: &TxId) -> Option<TxInfo> {
        txpool.read().await.txs().get(hash).cloned()
    }

    /// find all dependent tx and return them with requested dependencies in one list sorted by Price.
    pub async fn find_dependent(
        txpool: &RwLock<Self>,
        hashes: &[TxId],
    ) -> Vec<ArcPoolTx> {
        let mut seen = HashMap::new();
        {
            let pool = txpool.read().await;
            for hash in hashes {
                if let Some(tx) = pool.txs().get(hash) {
                    pool.dependency().find_dependent(
                        tx.tx().clone(),
                        &mut seen,
                        pool.txs(),
                    );
                }
            }
        }
        let mut list: Vec<ArcPoolTx> = seen.into_iter().map(|(_, tx)| tx).collect();
        // sort from high to low price
        list.sort_by_key(|tx| Reverse(tx.price()));

        list
    }

    /// Iterate over `hashes` and return all hashes that we don't have.
    pub async fn filter_by_negative(txpool: &RwLock<Self>, tx_ids: &[TxId]) -> Vec<TxId> {
        let mut res = Vec::new();
        let pool = txpool.read().await;
        for tx_id in tx_ids {
            if pool.txs().get(tx_id).is_none() {
                res.push(*tx_id)
            }
        }
        res
    }

    /// The amount of gas in all includable transactions combined
    pub async fn consumable_gas(txpool: &RwLock<Self>) -> u64 {
        let pool = txpool.read().await;
        pool.by_hash.values().map(|tx| tx.limit()).sum()
    }

    /// Return all sorted transactions that are includable in next block.
    /// This is going to be heavy operation, use it only when needed.
    pub async fn includable(txpool: &RwLock<Self>) -> Vec<ArcPoolTx> {
        let pool = txpool.read().await;
        pool.sorted_includable()
    }

    /// When block is updated we need to receive all spend outputs and remove them from txpool.
    pub async fn block_update(
        txpool: &RwLock<Self>,
        block: Arc<FuelBlock>,
        // spend_outputs: [Input], added_outputs: [AddedOutputs]
    ) {
        let mut guard = txpool.write().await;
        // TODO https://github.com/FuelLabs/fuel-core/issues/465

        for tx in block.transactions() {
            let _removed = guard.remove_by_tx_id(&tx.id());
        }
    }

    /// remove transaction from pool needed on user demand. Low priority
    pub async fn remove(
        txpool: &RwLock<Self>,
        broadcast: broadcast::Sender<TxStatusBroadcast>,
        tx_ids: &[TxId],
    ) -> Vec<ArcPoolTx> {
        let mut removed = Vec::new();
        for tx_id in tx_ids {
            let rem = { txpool.write().await.remove_by_tx_id(tx_id) };
            removed.extend(rem.into_iter());
        }
        for tx in &removed {
            let _ = broadcast.send(TxStatusBroadcast {
                tx: tx.clone(),
                status: TxStatus::SqueezedOut {
                    reason: Error::Removed,
                },
            });
        }
        removed
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::{
        test_helpers::{
            create_output_and_input,
            random_predicate,
            setup_coin,
            TEST_COIN_AMOUNT,
        },
        txpool::tests::helpers::{
            create_coin_output,
            create_contract_input,
            create_contract_output,
        },
        Error,
        MockDb,
    };
    use fuel_core_interfaces::{
        common::{
            fuel_crypto::rand::{
                rngs::StdRng,
                SeedableRng,
            },
            fuel_storage::StorageAsMut,
            fuel_tx::{
                AssetId,
                Output,
                TransactionBuilder,
                UtxoId,
            },
        },
        db::{
            Coins,
            Messages,
        },
        model::CoinStatus,
    };
    use std::{
        cmp::Reverse,
        str::FromStr,
        sync::Arc,
        vec,
    };

    mod helpers {
        use crate::types::TxId;
        use fuel_core_interfaces::{
            common::{
                fuel_tx::{
                    Contract,
                    ContractId,
                    Input,
                    Output,
                    UtxoId,
                },
                prelude::{
                    Opcode,
                    Word,
                },
            },
            model::{
                BlockHeight,
                Message,
            },
        };

        pub(crate) fn create_message_predicate_from_message(
            amount: Word,
            spent_block: Option<BlockHeight>,
        ) -> (Message, Input) {
            let predicate = vec![Opcode::RET(1)].into_iter().collect::<Vec<u8>>();
            let message = Message {
                sender: Default::default(),
                recipient: Input::predicate_owner(&predicate),
                nonce: 0,
                amount,
                data: vec![],
                da_height: Default::default(),
                fuel_block_spend: spent_block,
            };

            (
                message.clone(),
                Input::message_predicate(
                    message.id(),
                    message.sender,
                    Input::predicate_owner(&predicate),
                    message.amount,
                    message.nonce,
                    message.data,
                    predicate,
                    Default::default(),
                ),
            )
        }

        pub(crate) fn create_coin_output() -> Output {
            Output::Coin {
                amount: Default::default(),
                to: Default::default(),
                asset_id: Default::default(),
            }
        }

        pub(crate) fn create_contract_input(tx_id: TxId, output_index: u8) -> Input {
            Input::Contract {
                utxo_id: UtxoId::new(tx_id, output_index),
                balance_root: Default::default(),
                state_root: Default::default(),
                tx_pointer: Default::default(),
                contract_id: Default::default(),
            }
        }

        pub(crate) fn create_contract_output(contract_id: ContractId) -> Output {
            Output::ContractCreated {
                contract_id,
                state_root: Contract::default_state_root(),
            }
        }
    }

    #[tokio::test]
    async fn simple_insertion() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx, &db)
            .await
            .expect("Transaction should be OK, got Err");
    }

    #[tokio::test]
    async fn simple_dependency_tx1_tx2() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let (output, unset_input) = create_output_and_input(&mut rng, 1);
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(1)
                .add_input(gas_coin)
                .add_output(output)
                .finalize()
                .into(),
        );

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let input = unset_input.into_input(UtxoId::new(tx1.id(), 0));

        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(1)
                .add_input(input)
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1, &db)
            .await
            .expect("Tx1 should be OK, got Err");
        txpool
            .insert_inner(tx2, &db)
            .await
            .expect("Tx2 dependent should be OK, got Err");
    }

    #[tokio::test]
    async fn faulty_t2_collided_on_contract_id_from_tx1() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let contract_id = ContractId::from_str(
            "0x0000000000000000000000000000000000000000000000000000000000000100",
        )
        .unwrap();

        // contract creation tx
        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let (output, unset_input) = create_output_and_input(&mut rng, 10);
        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::create(
                Default::default(),
                Default::default(),
                Default::default(),
            )
            .gas_price(10)
            .add_input(gas_coin)
            .add_output(create_contract_output(contract_id))
            .add_output(output)
            .finalize()
            .into(),
        );

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let input = unset_input.into_input(UtxoId::new(tx.id(), 1));

        // attempt to insert a different creation tx with a valid dependency on the first tx,
        // but with a conflicting output contract id
        let tx_faulty: Arc<Transaction> = Arc::new(
            TransactionBuilder::create(
                Default::default(),
                Default::default(),
                Default::default(),
            )
            .gas_price(9)
            .add_input(gas_coin)
            .add_input(input)
            .add_output(create_contract_output(contract_id))
            .add_output(output)
            .finalize()
            .into(),
        );

        txpool
            .insert_inner(tx, &db)
            .await
            .expect("Tx1 should be Ok, got Err");

        let err = txpool
            .insert_inner(tx_faulty, &db)
            .await
            .expect_err("Tx2 should be Err, got Ok");
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedCollisionContractId(id)) if id == &contract_id
        ));
    }

    #[tokio::test]
    async fn fail_to_insert_tx_with_dependency_on_invalid_utxo_type() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let contract_id = ContractId::from_str(
            "0x0000000000000000000000000000000000000000000000000000000000000100",
        )
        .unwrap();
        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx_faulty: Arc<Transaction> = Arc::new(
            TransactionBuilder::create(
                Default::default(),
                Default::default(),
                Default::default(),
            )
            .add_input(gas_coin)
            .add_output(create_contract_output(contract_id))
            .finalize()
            .into(),
        );

        // create a second transaction with utxo id referring to
        // the wrong type of utxo (contract instead of coin)
        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(0)
                .add_input(random_predicate(
                    &mut rng,
                    AssetId::BASE,
                    TEST_COIN_AMOUNT,
                    Some(UtxoId::new(tx_faulty.id(), 0)),
                ))
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx_faulty.clone(), &db)
            .await
            .expect("Tx1 should be Ok, got Err");

        let err = txpool
            .insert_inner(tx, &db)
            .await
            .expect_err("Tx2 should be Err, got Ok");
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedInputUtxoIdNotExisting(id)) if id == &UtxoId::new(tx_faulty.id(), 0)
        ));
    }

    #[tokio::test]
    async fn not_inserted_known_tx() {
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let tx: Arc<Transaction> =
            Arc::new(TransactionBuilder::script(vec![], vec![]).finalize().into());

        txpool
            .insert_inner(tx.clone(), &db)
            .await
            .expect("Tx1 should be Ok, got Err");

        let err = txpool
            .insert_inner(tx, &db)
            .await
            .expect_err("Second insertion of Tx1 should be Err, got Ok");
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedTxKnown)
        ));
    }

    #[tokio::test]
    async fn try_to_insert_tx2_missing_utxo() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, input) = setup_coin(&mut rng, None);
        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(input)
                .finalize()
                .into(),
        );

        let err = txpool
            .insert_inner(tx, &db)
            .await
            .expect_err("Tx should be Err, got Ok");
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedInputUtxoIdNotExisting(_))
        ));
    }

    #[tokio::test]
    async fn tx_try_to_use_spent_coin() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let mut db = MockDb::default();

        // put a spent coin into the database
        let (mut coin, input) = setup_coin(&mut rng, None);
        let utxo_id = *input.utxo_id().unwrap();
        coin.status = CoinStatus::Spent;
        db.storage::<Coins>().insert(&utxo_id, &coin).unwrap();

        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(input)
                .finalize()
                .into(),
        );

        // attempt to insert the tx with an already spent coin
        let err = txpool
            .insert_inner(tx, &db)
            .await
            .expect_err("Tx should be Err, got Ok");
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedInputUtxoIdSpent(id)) if id == &utxo_id
        ));
    }

    #[tokio::test]
    async fn higher_priced_tx_removes_lower_priced_tx() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, coin_input) = setup_coin(&mut rng, Some(&db));

        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(coin_input.clone())
                .finalize()
                .into(),
        );
        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(20)
                .add_input(coin_input)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1.clone(), &db)
            .await
            .expect("Tx1 should be Ok, got Err");

        let vec = txpool
            .insert_inner(tx2, &db)
            .await
            .expect("Tx2 should be Ok, got Err");
        assert_eq!(vec.removed[0].id(), tx1.id(), "Tx1 id should be removed");
    }

    #[tokio::test]
    async fn underpriced_tx1_not_included_coin_collision() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let (output, unset_input) = create_output_and_input(&mut rng, 10);
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(20)
                .add_input(gas_coin)
                .add_output(output)
                .finalize()
                .into(),
        );
        let input = unset_input.into_input(UtxoId::new(tx1.id(), 0));

        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(20)
                .add_input(input.clone())
                .finalize()
                .into(),
        );
        let tx3: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(input)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1.clone(), &db)
            .await
            .expect("Tx1 should be Ok, got Err");
        txpool
            .insert_inner(tx2.clone(), &db)
            .await
            .expect("Tx2 should be Ok, got Err");

        let err = txpool
            .insert_inner(tx3.clone(), &db)
            .await
            .expect_err("Tx3 should be Err, got Ok");
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedCollision(id, utxo_id)) if id == &tx2.id() && utxo_id == &UtxoId::new(tx1.id(), 0)
        ));
    }

    #[tokio::test]
    async fn overpriced_tx_contract_input_not_inserted() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, gas_funds) = setup_coin(&mut rng, Some(&db));
        let contract_id = ContractId::default();
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::create(
                Default::default(),
                Default::default(),
                Default::default(),
            )
            .gas_price(10)
            .add_input(gas_funds)
            .add_output(create_contract_output(contract_id))
            .finalize()
            .into(),
        );

        let (_, gas_funds) = setup_coin(&mut rng, Some(&db));
        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(11)
                .add_input(gas_funds)
                .add_input(create_contract_input(
                    Default::default(),
                    Default::default(),
                ))
                .add_output(Output::contract(1, Default::default(), Default::default()))
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1, &db)
            .await
            .expect("Tx1 should be Ok, got err");

        let err = txpool
            .insert_inner(tx2, &db)
            .await
            .expect_err("Tx2 should be Err, got Ok");
        assert!(
            matches!(
                err.downcast_ref::<Error>(),
                Some(Error::NotInsertedContractPricedLower(id)) if id == &contract_id
            ),
            "wrong err {:?}",
            err
        );
    }

    #[tokio::test]
    async fn dependent_contract_input_inserted() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let contract_id = ContractId::default();
        let (_, gas_funds) = setup_coin(&mut rng, Some(&db));
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::create(
                Default::default(),
                Default::default(),
                Default::default(),
            )
            .gas_price(10)
            .add_input(gas_funds)
            .add_output(create_contract_output(contract_id))
            .finalize()
            .into(),
        );

        let (_, gas_funds) = setup_coin(&mut rng, Some(&db));
        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(gas_funds)
                .add_input(create_contract_input(
                    Default::default(),
                    Default::default(),
                ))
                .add_output(Output::contract(1, Default::default(), Default::default()))
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1, &db)
            .await
            .expect("Tx1 should be Ok, got Err");
        txpool
            .insert_inner(tx2, &db)
            .await
            .expect("Tx2 should be Ok, got Err");
    }

    #[tokio::test]
    async fn more_priced_tx3_removes_tx1_and_dependent_tx2() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));

        let (output, unset_input) = create_output_and_input(&mut rng, 10);
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(gas_coin.clone())
                .add_output(output)
                .finalize()
                .into(),
        );
        let input = unset_input.into_input(UtxoId::new(tx1.id(), 0));

        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(9)
                .add_input(input.clone())
                .finalize()
                .into(),
        );
        let tx3: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(20)
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1.clone(), &db)
            .await
            .expect("Tx1 should be OK, got Err");
        txpool
            .insert_inner(tx2.clone(), &db)
            .await
            .expect("Tx2 should be OK, got Err");
        let vec = txpool
            .insert_inner(tx3.clone(), &db)
            .await
            .expect("Tx3 should be OK, got Err");
        assert_eq!(
            vec.removed.len(),
            2,
            "Tx1 and Tx2 should be removed:{:?}",
            vec
        );
        assert_eq!(vec.removed[0].id(), tx1.id(), "Tx1 id should be removed");
        assert_eq!(vec.removed[1].id(), tx2.id(), "Tx2 id should be removed");
    }

    #[tokio::test]
    async fn tx_limit_hit() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Config {
            max_tx: 1,
            ..Default::default()
        });
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(gas_coin)
                .add_output(create_coin_output())
                .finalize()
                .into(),
        );
        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1, &db)
            .await
            .expect("Tx1 should be Ok, got Err");

        let err = txpool
            .insert_inner(tx2, &db)
            .await
            .expect_err("Tx2 should be Err, got Ok");
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedLimitHit)
        ));
    }

    #[tokio::test]
    async fn tx_depth_hit() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Config {
            max_depth: 2,
            ..Default::default()
        });
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let (output, unset_input) = create_output_and_input(&mut rng, 10_000);
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(gas_coin)
                .add_output(output)
                .finalize()
                .into(),
        );

        let input = unset_input.into_input(UtxoId::new(tx1.id(), 0));
        let (output, unset_input) = create_output_and_input(&mut rng, 5_000);
        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(input)
                .add_output(output)
                .finalize()
                .into(),
        );

        let input = unset_input.into_input(UtxoId::new(tx2.id(), 0));
        let tx3: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(input)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1, &db)
            .await
            .expect("Tx1 should be OK, got Err");
        txpool
            .insert_inner(tx2, &db)
            .await
            .expect("Tx2 should be OK, got Err");

        let err = txpool
            .insert_inner(tx3, &db)
            .await
            .expect_err("Tx3 should be Err, got Ok");
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedMaxDepth)
        ));
    }

    #[tokio::test]
    async fn sorted_out_tx1_2_4() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(9)
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx3: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(20)
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1.clone(), &db)
            .await
            .expect("Tx1 should be Ok, got Err");
        txpool
            .insert_inner(tx2.clone(), &db)
            .await
            .expect("Tx2 should be Ok, got Err");
        txpool
            .insert_inner(tx3.clone(), &db)
            .await
            .expect("Tx4 should be Ok, got Err");

        let txs = txpool.sorted_includable();

        assert_eq!(txs.len(), 3, "Should have 3 txs");
        assert_eq!(txs[0].id(), tx3.id(), "First should be tx3");
        assert_eq!(txs[1].id(), tx1.id(), "Second should be tx1");
        assert_eq!(txs[2].id(), tx2.id(), "Third should be tx2");
    }

    #[tokio::test]
    async fn find_dependent_tx1_tx2() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Default::default());
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let (output, unset_input) = create_output_and_input(&mut rng, 10_000);
        let tx1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(11)
                .add_input(gas_coin)
                .add_output(output)
                .finalize()
                .into(),
        );

        let input = unset_input.into_input(UtxoId::new(tx1.id(), 0));
        let (output, unset_input) = create_output_and_input(&mut rng, 7_500);
        let tx2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(input)
                .add_output(output)
                .finalize()
                .into(),
        );

        let input = unset_input.into_input(UtxoId::new(tx2.id(), 0));
        let tx3: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(9)
                .add_input(input)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx1.clone(), &db)
            .await
            .expect("Tx0 should be Ok, got Err");
        txpool
            .insert_inner(tx2.clone(), &db)
            .await
            .expect("Tx1 should be Ok, got Err");
        let tx3_result = txpool
            .insert_inner(tx3.clone(), &db)
            .await
            .expect("Tx2 should be Ok, got Err");

        let mut seen = HashMap::new();
        txpool
            .dependency()
            .find_dependent(tx3_result.inserted, &mut seen, txpool.txs());

        let mut list: Vec<ArcPoolTx> = seen.into_iter().map(|(_, tx)| tx).collect();
        // sort from high to low price
        list.sort_by_key(|tx| Reverse(tx.price()));
        assert_eq!(list.len(), 3, "We should have three items");
        assert_eq!(list[0].id(), tx1.id(), "Tx1 should be first.");
        assert_eq!(list[1].id(), tx2.id(), "Tx2 should be second.");
        assert_eq!(list[2].id(), tx3.id(), "Tx3 should be third.");
    }

    #[tokio::test]
    async fn tx_at_least_min_gas_price_is_insertable() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Config {
            min_gas_price: 10,
            ..Default::default()
        });
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        txpool
            .insert_inner(tx, &db)
            .await
            .expect("Tx should be Ok, got Err");
    }

    #[tokio::test]
    async fn tx_below_min_gas_price_is_not_insertable() {
        let mut rng = StdRng::seed_from_u64(0);
        let mut txpool = TxPool::new(Config {
            min_gas_price: 11,
            ..Default::default()
        });
        let db = MockDb::default();

        let (_, gas_coin) = setup_coin(&mut rng, Some(&db));
        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(10)
                .add_input(gas_coin)
                .finalize()
                .into(),
        );

        let err = txpool
            .insert_inner(tx, &db)
            .await
            .expect_err("expected insertion failure");
        assert!(matches!(
            err.root_cause().downcast_ref::<Error>().unwrap(),
            Error::NotInsertedGasPriceTooLow
        ));
    }

    #[tokio::test]
    async fn tx_inserted_into_pool_when_input_message_id_exists_in_db() {
        let (message, input) = helpers::create_message_predicate_from_message(5000, None);

        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(input)
                .finalize()
                .into(),
        );

        let mut db = MockDb::default();
        db.storage::<Messages>()
            .insert(&message.id(), &message)
            .unwrap();
        let mut txpool = TxPool::new(Default::default());

        txpool
            .insert_inner(tx.clone(), &db)
            .await
            .expect("should succeed");

        let tx_info = TxPool::find_one(&RwLock::new(txpool), &tx.id())
            .await
            .unwrap();
        assert_eq!(tx_info.tx().id(), tx.id());
    }

    #[tokio::test]
    async fn tx_rejected_when_input_message_id_is_spent() {
        let (message, input) =
            helpers::create_message_predicate_from_message(5_000, Some(1u64.into()));

        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(input)
                .finalize()
                .into(),
        );

        let mut db = MockDb::default();
        db.storage::<Messages>()
            .insert(&message.id(), &message)
            .unwrap();
        let mut txpool = TxPool::new(Default::default());

        let err = txpool
            .insert_inner(tx.clone(), &db)
            .await
            .expect_err("should fail");

        // check error
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedInputMessageIdSpent(msg_id)) if msg_id == &message.id()
        ));
    }

    #[tokio::test]
    async fn tx_rejected_from_pool_when_input_message_id_does_not_exist_in_db() {
        let (message, input) = helpers::create_message_predicate_from_message(5000, None);
        let tx: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .add_input(input)
                .finalize()
                .into(),
        );

        let db = MockDb::default();
        // Do not insert any messages into the DB to ensure there is no matching message for the
        // tx.

        let mut txpool = TxPool::new(Default::default());

        let err = txpool
            .insert_inner(tx.clone(), &db)
            .await
            .expect_err("should fail");

        // check error
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedInputMessageUnknown(msg_id)) if msg_id == &message.id()
        ));
    }

    #[tokio::test]
    async fn tx_rejected_from_pool_when_gas_price_is_lower_than_another_tx_with_same_message_id(
    ) {
        let message_amount = 10_000;
        let gas_price_high = 2u64;
        let gas_price_low = 1u64;
        let (message, conflicting_message_input) =
            helpers::create_message_predicate_from_message(message_amount, None);

        let tx_high: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(gas_price_high)
                .add_input(conflicting_message_input.clone())
                .finalize()
                .into(),
        );

        let tx_low: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(gas_price_low)
                .add_input(conflicting_message_input)
                .finalize()
                .into(),
        );

        let mut db = MockDb::default();
        db.storage::<Messages>()
            .insert(&message.id(), &message)
            .unwrap();

        let mut txpool = TxPool::new(Default::default());

        // Insert a tx for the message id with a high gas amount
        txpool
            .insert_inner(tx_high.clone(), &db)
            .await
            .expect("expected successful insertion");

        // Insert a tx for the message id with a low gas amount
        // Because the new transaction's id matches an existing transaction, we compare the gas
        // prices of both the new and existing transactions. Since the existing transaction's gas
        // price is higher, we must now reject the new transaction.
        let err = txpool
            .insert_inner(tx_low.clone(), &db)
            .await
            .expect_err("expected failure");

        // check error
        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::NotInsertedCollisionMessageId(tx_id, msg_id)) if tx_id == &tx_high.id() && msg_id == &message.id()
        ));
    }

    #[tokio::test]
    async fn higher_priced_tx_squeezes_out_lower_priced_tx_with_same_message_id() {
        let message_amount = 10_000;
        let gas_price_high = 2u64;
        let gas_price_low = 1u64;
        let (message, conflicting_message_input) =
            helpers::create_message_predicate_from_message(message_amount, None);

        // Insert a tx for the message id with a low gas amount
        let tx_low: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(gas_price_low)
                .add_input(conflicting_message_input.clone())
                .finalize()
                .into(),
        );

        let mut db = MockDb::default();
        db.storage::<Messages>()
            .insert(&message.id(), &message)
            .unwrap();

        let mut txpool = TxPool::new(Default::default());

        txpool
            .insert_inner(tx_low.clone(), &db)
            .await
            .expect("should succeed");

        // Insert a tx for the message id with a high gas amount
        // Because the new transaction's id matches an existing transaction, we compare the gas
        // prices of both the new and existing transactions. Since the existing transaction's gas
        // price is lower, we accept the new transaction and squeeze out the old transaction.
        let tx_high: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(gas_price_high)
                .add_input(conflicting_message_input)
                .finalize()
                .into(),
        );

        let squeezed_out_txs = txpool
            .insert_inner(tx_high.clone(), &db)
            .await
            .expect("should succeed");

        assert_eq!(squeezed_out_txs.removed.len(), 1);
        assert_eq!(squeezed_out_txs.removed[0].id(), tx_low.id());
    }

    #[tokio::test]
    async fn message_of_squeezed_out_tx_can_be_resubmitted_at_lower_gas_price() {
        // tx1 (message 1, message 2) gas_price 2
        // tx2 (message 1) gas_price 3
        //   squeezes tx1 with higher gas price
        // tx3 (message 2) gas_price 1
        //   works since tx1 is no longer part of txpool state even though gas price is less

        let (message_1, message_input_1) =
            helpers::create_message_predicate_from_message(10_000, None);
        let (message_2, message_input_2) =
            helpers::create_message_predicate_from_message(20_000, None);

        // Insert a tx for the message id with a low gas amount
        let tx_1: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(2)
                .add_input(message_input_1.clone())
                .add_input(message_input_2.clone())
                .finalize()
                .into(),
        );

        let tx_2: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(3)
                .add_input(message_input_1.clone())
                .finalize()
                .into(),
        );

        let tx_3: Arc<Transaction> = Arc::new(
            TransactionBuilder::script(vec![], vec![])
                .gas_price(1)
                .add_input(message_input_2.clone())
                .finalize()
                .into(),
        );

        let mut db = MockDb::default();
        db.storage::<Messages>()
            .insert(&message_1.id(), &message_1)
            .unwrap();
        db.storage::<Messages>()
            .insert(&message_2.id(), &message_2)
            .unwrap();
        let mut txpool = TxPool::new(Default::default());

        txpool
            .insert_inner(tx_1, &db)
            .await
            .expect("should succeed");

        txpool
            .insert_inner(tx_2, &db)
            .await
            .expect("should succeed");

        txpool
            .insert_inner(tx_3, &db)
            .await
            .expect("should succeed");
    }
}
