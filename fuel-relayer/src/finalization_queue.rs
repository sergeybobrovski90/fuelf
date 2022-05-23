use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use ethers_core::types::{Log, H160};
use ethers_providers::Middleware;
use fuel_core_interfaces::{
    model::{BlockHeight, DaBlockHeight, SealedFuelBlock},
    relayer::{RelayerDb, StakingDiff},
};
use fuel_tx::{Address, Bytes32};
use tracing::{debug, error, info, warn};

use crate::{
    log::{AssetDepositLog, EthEventLog},
    pending_blocks::PendingBlocks,
    validators::Validators,
};

pub struct FinalizationQueue {
    /// Pending stakes/assets/withdrawals. Before they are finalized
    pending: VecDeque<DaBlockDiff>,
    /// Revert on eth are reported as list of reverted logs in order of Block2Log1,Block2Log2,Block1Log1,Block2Log2.
    /// So when applying multiple block reverts it is good to mind the order.
    bundled_removed_eth_events: Vec<(DaBlockHeight, Vec<EthEventLog>)>,
    /// finalized fuel block
    finalized_da_height: DaBlockHeight,
    /// Pending block handling
    blocks: PendingBlocks,
    /// Current validator set
    validators: Validators,
}

/// Pending diff between FuelBlocks
#[derive(Clone, Debug, Default)]
pub struct DaBlockDiff {
    /// da block height
    pub da_height: DaBlockHeight,
    /// Validator stake deposit and withdrawel.
    pub validators: HashMap<Address, Option<Address>>,
    // Delegation diff contains new delegation list, if we did just withdrawal option will be None.
    pub delegations: HashMap<Address, Option<HashMap<Address, u64>>>,
    /// erc-20 pending deposit.
    pub assets: HashMap<Bytes32, AssetDepositLog>,
}

impl DaBlockDiff {
    pub fn new(da_height: u64) -> Self {
        Self {
            da_height,
            validators: HashMap::new(),
            delegations: HashMap::new(),
            assets: HashMap::new(),
        }
    }
}

impl FinalizationQueue {
    pub fn new(
        chain_id: u64,
        contract_address: H160,
        private_key: &[u8],
        last_commited_finalized_fuel_height: BlockHeight,
    ) -> Self {
        let blocks = PendingBlocks::new(
            chain_id,
            contract_address,
            private_key,
            last_commited_finalized_fuel_height,
        );
        Self {
            blocks,
            pending: VecDeque::new(),
            validators: Validators::default(),
            bundled_removed_eth_events: Vec::new(),
            finalized_da_height: 0,
        }
    }

    pub async fn load_validators(&mut self, db: &dyn RelayerDb) {
        self.validators.load(db).await
    }

    pub async fn get_validators(
        &mut self,
        da_height: DaBlockHeight,
    ) -> Option<HashMap<Address, (u64, Option<Address>)>> {
        self.validators.get(da_height).await
    }

    pub fn clear(&mut self) {
        self.pending.clear()
    }

    /// Bundle all removed events to apply them in same time when all of them are flushed.
    fn bundle_removed_events(&mut self, event: EthEventLog, eth_block: u64) {
        // agregate all removed events before reverting them.
        // check if we have pending block for removal
        if let Some((last_eth_block, list)) = self.bundled_removed_eth_events.last_mut() {
            // check if last pending block is same as log event that we received.
            if *last_eth_block == eth_block {
                list.push(event)
            } else {
                // if block number differs just push new block.
                self.bundled_removed_eth_events
                    .push((eth_block, vec![event]));
            }
        } else {
            // if there are not pending block for removal just add it.
            self.bundled_removed_eth_events
                .push((eth_block, vec![event]));
        }
    }

    /// propagate new fuel block to pending_blocks
    pub fn handle_fuel_block(&mut self, block: &Arc<SealedFuelBlock>) {
        self.blocks.set_chain_height(block.header.height)
    }

    /// propagate new created fuel block to pending_blocks
    pub async fn handle_created_fuel_block<P>(
        &mut self,
        block: &Arc<SealedFuelBlock>,
        db: &mut dyn RelayerDb,
        provider: &Arc<P>,
    ) where
        P: Middleware + 'static,
    {
        self.blocks.commit(block.header.height, db, provider).await;
    }

    pub async fn append_eth_logs(&mut self, logs: Vec<Log>) {
        for log in logs {
            self.append_eth_log(log).await;
        }
    }

    /// Handle eth log events
    pub async fn append_eth_log(&mut self, log: Log) {
        let event = EthEventLog::try_from(&log);
        if let Err(err) = event {
            warn!(target:"relayer", "Eth Event not formated properly:{}",err);
            return;
        }
        if log.block_number.is_none() {
            error!(target:"relayer", "Block number not found in eth log");
            return;
        }
        let removed = log.removed.unwrap_or(false);
        let eth_block = log.block_number.unwrap().as_u64();
        let event = event.unwrap();
        debug!("append inbound log:{:?}", event);
        // bundle removed events and return
        if removed {
            self.bundle_removed_events(event, eth_block);
            return;
        }
        // apply all reverted event
        if !self.bundled_removed_eth_events.is_empty() {
            info!(
                "Reorg happened on ethereum. Reverting {} logs",
                self.bundled_removed_eth_events.len()
            );

            let mut lowest_removed_da_height = u64::MAX;

            for (da_height, events) in
                std::mem::take(&mut self.bundled_removed_eth_events).into_iter()
            {
                lowest_removed_da_height = u64::min(lowest_removed_da_height, da_height);
                // mark all removed pending block commits as reverted.
                for event in events {
                    if let EthEventLog::FuelBlockCommited { block_root, height } = event {
                        self.blocks
                            .handle_block_commit(block_root, height.into(), da_height, true);
                    }
                }
            }
            // remove all blocks that were reverted. In best case those blocks heights and events are going
            // to be reinserted in append eth events.
            self.pending
                .retain(|diff| diff.da_height < lowest_removed_da_height);
        }
        // apply new event to pending queue
        self.append_da_events(event, eth_block).await;
    }

    /// At begining we will ignore all event until event for new fuel block commit commes
    /// after that syncronization can start.
    async fn append_da_events(&mut self, fuel_event: EthEventLog, da_height: u64) {
        if let Some(front) = self.pending.back() {
            if front.da_height != da_height {
                self.pending.push_back(DaBlockDiff::new(da_height))
            }
        } else {
            self.pending.push_back(DaBlockDiff::new(da_height))
        }
        let last_diff = self.pending.back_mut().unwrap();
        match fuel_event {
            EthEventLog::AssetDeposit(deposit) => {
                last_diff.assets.insert(deposit.deposit_nonce, deposit);
            }
            EthEventLog::Deposit { .. } => {
                // It is fine to do nothing. This is only related to contract,
                // only possible usage for this is as additional information for user.
            }
            EthEventLog::Withdrawal { withdrawer, .. } => {
                last_diff.delegations.insert(withdrawer, None);
            }
            EthEventLog::Delegation {
                delegator,
                delegates,
                amounts,
            } => {
                let delegates: HashMap<_, _> = delegates
                    .iter()
                    .zip(amounts.iter())
                    .map(|(f, s)| (*f, *s))
                    .collect();
                last_diff.delegations.insert(delegator, Some(delegates));
            }
            EthEventLog::ValidatorRegistration {
                staking_key,
                consensus_key,
            } => {
                last_diff
                    .validators
                    .insert(staking_key, Some(consensus_key));
            }
            EthEventLog::ValidatorUnregistration { staking_key } => {
                last_diff.validators.insert(staking_key, None);
            }
            EthEventLog::FuelBlockCommited { height, block_root } => {
                self.blocks
                    .handle_block_commit(block_root, (height).into(), da_height, false);
            }
            EthEventLog::Unknown => (),
        }
    }

    /// Used to commit da block diff to database.
    pub async fn commit_diffs(&mut self, db: &mut dyn RelayerDb, finalized_da_height: u64) {
        if self.finalized_da_height >= finalized_da_height {
            error!(
                "We received finalized height {} but we already have {}",
                finalized_da_height, self.finalized_da_height
            );
            return;
        }
        while let Some(diff) = self.pending.front_mut() {
            if diff.da_height > finalized_da_height {
                break;
            }
            info!("flush eth log:{:?} diff:{:?}", diff.da_height, diff);
            //TODO to be paranoid, recheck events got from eth client.

            // apply staking diffs
            db.insert_staking_diff(
                diff.da_height,
                &StakingDiff::new(diff.validators.clone(), diff.delegations.clone()),
            )
            .await;

            // append index of delegator so that we cross reference earliest delegation set
            for (delegate, _) in diff.delegations.iter() {
                db.append_delegate_index(delegate, diff.da_height).await;
            }

            // push finalized assets to db
            for (_, deposit) in diff.assets.iter() {
                db.insert_coin_deposit(deposit.into()).await
            }

            // insert height index into delegations.
            db.set_finalized_da_height(diff.da_height).await;

            // remove pending diff
            self.pending.pop_front();
        }

        let last_commited_fin_fuel_height = self.blocks.handle_da_finalization(finalized_da_height);

        db.set_last_commited_finalized_fuel_height(last_commited_fin_fuel_height)
            .await;
        self.finalized_da_height = finalized_da_height;
        // bump validator set to last finalized block
        self.validators
            .bump_set_to_da_height(finalized_da_height, db)
            .await
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::log::tests::*;
    use fuel_core_interfaces::db::helpers::DummyDb;
    use fuel_types::{Address, AssetId};
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    #[tokio::test]
    pub async fn check_token_deposits_on_multiple_eth_blocks() {
        let mut rng = StdRng::seed_from_u64(3020);

        let acc1: Address = rng.gen();
        let token1 = AssetId::zeroed();
        let nonce1: Bytes32 = rng.gen();
        let nonce2: Bytes32 = rng.gen();
        let nonce3: Bytes32 = rng.gen();

        let mut queue = FinalizationQueue::new(
            0,
            H160::zero(),
            &(hex::decode("79afbf7147841fca72b45a1978dd7669470ba67abbe5c220062924380c9c364b")
                .unwrap()),
            BlockHeight::from(0u64),
        );

        let deposit1 = eth_log_asset_deposit(0, acc1, token1, 0, 10, nonce1, 0);
        let deposit2 = eth_log_asset_deposit(1, acc1, token1, 1, 20, nonce2, 0);
        let deposit3 = eth_log_asset_deposit(1, acc1, token1, 1, 40, nonce3, 0);

        let deposit1_db = EthEventLog::try_from(&deposit1).unwrap();
        let deposit2_db = EthEventLog::try_from(&deposit2).unwrap();
        let deposit3_db = EthEventLog::try_from(&deposit3).unwrap();

        queue
            .append_eth_logs(vec![deposit1, deposit2, deposit3])
            .await;

        let diff1 = queue.pending[0].clone();
        let diff2 = queue.pending[1].clone();

        if let EthEventLog::AssetDeposit(deposit) = &deposit1_db {
            assert_eq!(
                diff1.assets.get(&nonce1),
                Some(deposit),
                "Deposit 1 not valid"
            );
        }
        if let EthEventLog::AssetDeposit(deposit) = &deposit2_db {
            assert_eq!(
                diff2.assets.get(&nonce2),
                Some(deposit),
                "Deposit 2 not valid"
            );
        }
        if let EthEventLog::AssetDeposit(deposit) = &deposit3_db {
            assert_eq!(
                diff2.assets.get(&nonce3),
                Some(deposit),
                "Deposit 3 not valid"
            );
        }
    }

    #[tokio::test]
    pub async fn check_validator_registration_unregistration() {
        let mut rng = StdRng::seed_from_u64(3020);
        let val1: Address = rng.gen();
        let cons1: Address = rng.gen();
        let val2: Address = rng.gen();
        let cons2: Address = rng.gen();

        let mut queue = FinalizationQueue::new(
            0,
            H160::zero(),
            &(hex::decode("79afbf7147841fca72b45a1978dd7669470ba67abbe5c220062924380c9c364b")
                .unwrap()),
            BlockHeight::from(0u64),
        );

        let val1_register = eth_log_validator_registration(0, val1, cons1);
        let val2_register = eth_log_validator_registration(0, val2, cons2);
        let val1_unregister = eth_log_validator_unregistration(1, val1);

        queue
            .append_eth_logs(vec![val1_register, val2_register, val1_unregister])
            .await;

        let diff1 = queue.pending[0].clone();
        let diff2 = queue.pending[1].clone();
        assert_eq!(
            diff1.validators.get(&val1),
            Some(&Some(cons1)),
            "Val1 registered cons1"
        );
        assert_eq!(
            diff1.validators.get(&val2),
            Some(&Some(cons2)),
            "Val1 registered cons2"
        );

        assert_eq!(
            diff2.validators.get(&val1),
            Some(&None),
            "Val1 unregistered consensus key"
        );
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    pub async fn check_deposit_and_validator_finalization() {
        let mut rng = StdRng::seed_from_u64(3020);
        let val1: Address = rng.gen();
        let cons1: Address = rng.gen();
        let val2: Address = rng.gen();
        let cons2: Address = rng.gen();

        let acc1: Address = rng.gen();
        let token1 = AssetId::zeroed();
        let nonce1: Bytes32 = rng.gen();

        let mut queue = FinalizationQueue::new(
            0,
            H160::zero(),
            &(hex::decode("79afbf7147841fca72b45a1978dd7669470ba67abbe5c220062924380c9c364b")
                .unwrap()),
            BlockHeight::from(0u64),
        );

        let val1_register = eth_log_validator_registration(1, val1, cons1);
        let val2_register = eth_log_validator_registration(2, val2, cons2);
        let deposit1 = eth_log_asset_deposit(2, acc1, token1, 1, 40, nonce1, 0);
        let val1_unregister = eth_log_validator_unregistration(3, val1);

        queue
            .append_eth_logs(vec![
                val1_register,
                val2_register,
                deposit1,
                val1_unregister,
            ])
            .await;

        let mut db = DummyDb::filled();
        //let db_ref = &mut db as &mut dyn RelayerDb;

        queue.commit_diffs(&mut db, 1).await;
        assert_eq!(
            db.data.lock().validators.get(&val1),
            Some(&(0, Some(cons1))),
            "Val1 should be set"
        );

        assert_eq!(
            db.data.lock().validators.get(&val2),
            None,
            "Val2 shouldn't be found"
        );

        assert_eq!(
            db.data.lock().deposit_coin.len(),
            0,
            "asset is not finalized"
        );

        queue.commit_diffs(&mut db, 2).await;

        assert_eq!(
            db.data.lock().validators.get(&val2),
            Some(&(0, Some(cons2))),
            "Val2 should be set"
        );

        assert_eq!(
            db.data.lock().deposit_coin.len(),
            1,
            "asset should be finalized"
        );

        queue.commit_diffs(&mut db, 3).await;

        assert_eq!(
            db.data.lock().validators.get(&val1),
            Some(&(0, None)),
            "Val1 should be unregistered"
        );
        assert_eq!(
            db.data.lock().validators.get(&val2),
            Some(&(0, Some(cons2))),
            "Val2 should be registered"
        );
        assert_eq!(
            db.data.lock().deposit_coin.len(),
            1,
            "asset should stay finalized"
        );
    }
}
