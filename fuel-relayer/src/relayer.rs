use std::{
    cmp::min,
    collections::{HashMap, VecDeque},
    time::Duration,
};

use crate::{log::EthEventLog, Config};
use fuel_tx::Bytes32;
use fuel_types::{Address, Color, Word};
use log::{error, info, trace, warn};
use tokio::sync::{mpsc, oneshot};

use anyhow::Error;
use ethers_core::types::{Filter, Log, ValueOrArray};
use ethers_providers::{
    FilterWatcher, JsonRpcClient, Middleware, Provider, PubsubClient, StreamExt, SyncingStatus, Ws,
};
use fuel_core_interfaces::relayer::{RelayerDB, RelayerEvent, RelayerStatus, RelayerError};
///
pub struct Relayer {
    /// Pendning stakes/assets/withdrawals. Before they are finalized
    pending: VecDeque<PendingDiff>,
    /// finalized validator set
    finalized_validator_set: HashMap<Address, u64>,
    /// finalized fuel block
    finalized_fuel_block: u64,
    /// Current validator set
    current_validator_set: HashMap<Address, u64>,
    /// current fuel block
    current_fuel_block: u64,
    /// db connector to apply stake and token deposit
    db: Box<dyn RelayerDB>,
    /// Relayer Configuration
    config: Config,
    /// state of relayer
    status: RelayerStatus,
    // new fuel block notifier.
    receiver: mpsc::Receiver<RelayerEvent>,
    /// This is litlle bit hacky but because we relate validator staking with fuel commit block and not on eth block
    /// we need to be sure that we are taking proper order of those transactions
    /// Revert are reported as list of reverted logs in order of Block2Log1,Block2Log2,Block1Log1,Block2Log2.
    /// I checked this with infura endpoint.
    pending_removed_eth_events: Vec<(u64, Vec<EthEventLog>)>,
}

/// Pending diff between FuelBlocks
#[derive(Clone, Debug)]
pub struct PendingDiff {
    /// fuel block number,
    fuel_number: u64,
    /// eth block number, Represent when child number got included in what block.
    /// This means that when that block is finalized we are okay to commit this pending diff.
    eth_number: u64,
    /// Validator stake deposit and withdrawel.
    stake_diff: HashMap<Address, i64>,
    /// erc-20 pending deposit. deposit nonce.
    assets_deposited: HashMap<Bytes32, (Address, Color, Word)>,
}

impl PendingDiff {
    pub fn new(fuel_number: u64) -> Self {
        Self {
            fuel_number,
            eth_number: u64::MAX,
            stake_diff: HashMap::new(),
            assets_deposited: HashMap::new(),
        }
    }
    pub fn stake_diff(&self) -> &HashMap<Address, i64> {
        &self.stake_diff
    }
    pub fn assets_deposited(&self) -> &HashMap<Bytes32, (Address, Color, Word)> {
        &self.assets_deposited
    }
}

impl Relayer {
    pub fn new(
        config: Config,
        db: Box<dyn RelayerDB>,
        receiver: mpsc::Receiver<RelayerEvent>,
    ) -> Self {
        Self {
            config,
            db,
            pending: VecDeque::new(),
            finalized_validator_set: HashMap::new(),
            finalized_fuel_block: 0,
            current_validator_set: HashMap::new(),
            current_fuel_block: 0,
            status: RelayerStatus::EthIsSyncing,
            receiver,
            pending_removed_eth_events: Vec::new(),
        }
    }

    /// create provider that we use for communication with ethereum.
    pub async fn provider(uri: &str) -> Result<Provider<Ws>, Error> {
        let ws = Ws::connect(uri).await?;
        let provider = Provider::new(ws);
        Ok(provider)
    }

    /// Used in two places. On initial sync and when new fuel blocks is
    async fn apply_last_validator_diff(&mut self, current_eth_number: u64) {
        let finalized_eth_block = current_eth_number - self.config.eth_finality_slider();
        while let Some(diffs) = self.pending.back() {
            if diffs.eth_number < finalized_eth_block {
                break;
            }
            let mut stake_diff = HashMap::new();
            // apply diff to validator_set
            for (address, diff) in &diffs.stake_diff {
                let value = self.finalized_validator_set.entry(*address).or_insert(0);
                // we are okay to cast it, we dont expect that big of number to exist.
                *value = ((*value as i64) + diff) as u64;
                stake_diff.insert(*address, *value);
            }
            // push new value for changed validators to database
            self.db
                .insert_validator_changes(diffs.fuel_number, &stake_diff)
                .await;
            self.db.set_fuel_finalized_block(diffs.fuel_number).await;

            // push fanalized deposit to db
            let block_enabled_fuel_block = diffs.fuel_number + self.config.fuel_finality_slider();
            for (nonce, deposit) in diffs.assets_deposited.iter() {
                self.db
                    .insert_token_deposit(
                        *nonce,
                        block_enabled_fuel_block,
                        deposit.0,
                        deposit.1,
                        deposit.2,
                    )
                    .await
            }
            self.db.set_eth_finalized_block(finalized_eth_block).await;
            self.finalized_fuel_block = diffs.fuel_number;
            self.pending.pop_back();
        }
        self.db.set_eth_finalized_block(finalized_eth_block).await;
    }

    /// Initial syncing from ethereum logs into fuel database. It does overlapping syncronization and returns
    /// logs watcher with assurence that we didnt miss any events.
    async fn inital_sync<'a, P>(
        &mut self,
        provider: &'a Provider<P>,
    ) -> Result<FilterWatcher<'a, P, Log>, Error>
    where
        P: JsonRpcClient + PubsubClient,
    {
        // loop and wait for eth client to finish syncing
        loop {
            if self.status == RelayerStatus::Stop {
                return Err(RelayerError::Stoped.into());
            }
            if let SyncingStatus::IsFalse = provider.syncing().await? {
                // sleep for some time until eth client is synced
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            } else {
                break;
            }
        }

        let last_finalized_eth_block = std::cmp::max(
            self.config.eth_v2_contract_deployment(),
            self.db.get_eth_finalized_block().await,
        );
        // should be allways more then last finalized_eth_block
        let best_finalized_block =
            provider.get_block_number().await?.as_u64() - self.config.eth_finality_slider();

        // 1. sync from HardCoddedContractCreatingBlock->BestEthBlock-100)
        let step = 1000; // do some stats on optimal value
        for start in (last_finalized_eth_block..best_finalized_block).step_by(step) {
            let end = min(start + step as u64, best_finalized_block);

            // TODO  can be parallelized
            let logs = provider
                .get_logs(&Filter::new().from_block(start).to_block(end).address(
                    ValueOrArray::Array(self.config.eth_v2_contract_addresses().to_vec()),
                ))
                .await?;

            for eth_event in logs {
                let fuel_event = EthEventLog::try_from(&eth_event);
                if let Err(err) = fuel_event {
                    // not formated event from contract
                    error!(target:"relayer", "Eth Event not formated properly in inital sync:{}",err);
                    // just skip it for now.
                    continue;
                }
                let fuel_event = fuel_event.unwrap();
                self.append_eth_events(&fuel_event, eth_event.block_number.unwrap().as_u64())
                    .await;
            }
            // if there is more then two items in pending list flush first one.
            // Having two elements in this stage means that full fuel block is already processed and
            // we dont have reverts to dispute that.
            while self.pending.len() > 1 {
                // we are sending dummy eth block bcs we are sure that it is finalized
                self.apply_last_validator_diff(u64::MAX).await;
            }
        }

        // if there is no diffs it means we are at start of contract creating
        let last_diff = if self.pending.is_empty() {
            // set fuel num to zero and contract creating eth.
            PendingDiff::new(0)
        } else {
            // apply all pending changed.
            while self.pending.len() > 1 {
                // we are sending dummy eth block num bcs we are sure that it is finalized
                self.apply_last_validator_diff(u64::MAX).await;
            }
            self.pending.pop_front().unwrap()
        };

        // TODO probably not needed now. but after some time we will need to do sync to best block here.
        // it depends on how much time it is needed to tranverse first part of this function
        // and how much lag happened in meantime.

        let mut watcher: Option<FilterWatcher<_, _>>;
        let last_included_block = best_finalized_block;

        let mut best_block;
        loop {
            // 1. get best block and its hash sync over it, and push it over

            self.pending.clear();
            self.pending.push_front(last_diff.clone());

            best_block = provider.get_block_number().await?;
            // there is not get block latest from ethers so we need to do it in two steps to get hash

            let block = provider
                .get_block(best_block)
                .await?
                .expect("TODO handle me");
            let best_block_hash = block.hash.unwrap(); // it is okey to just unwrap

            // 2. sync overlap from LastIncludedEthBlock-> BestEthBlock) they are saved in dequeue.
            let logs = provider
                .get_logs(
                    &Filter::new()
                        .from_block(last_included_block)
                        .to_block(best_block)
                        .address(ValueOrArray::Array(
                            self.config.eth_v2_contract_addresses().to_vec(),
                        )),
                )
                .await?;

            for eth_event in logs {
                let fuel_event = EthEventLog::try_from(&eth_event);
                if let Err(err) = fuel_event {
                    // not formated event from contract
                    error!(target:"relayer", "Eth Event not formated properly in inital sync:{}",err);
                    // just skip it for now.
                    continue;
                }
                let fuel_event = fuel_event.unwrap();
                self.append_eth_events(&fuel_event, eth_event.block_number.unwrap().as_u64())
                    .await;
            }

            // 3. Start listening to eth events
            let eth_log_filter = Filter::new().address(ValueOrArray::Array(
                self.config.eth_v2_contract_addresses().to_vec(),
            ));
            watcher = Some(provider.watch(&eth_log_filter).await.expect("TO WORK"));
            // sleep for 300ms just to be sure that our watcher is registered and started receiving events
            tokio::time::sleep(Duration::from_millis(300)).await;

            // 4. Check if our LastIncludedEthBlock is same as BestEthBlock
            if best_block == provider.get_block_number().await?
                && best_block_hash == provider.get_block(best_block).await?.unwrap().hash.unwrap()
            {
                // block number and hash are same as before starting watcher over logs.
                // we are safe to continue.
                break;
            }
            // If not the same, stop listening to events and do 2,3,4 steps again.
            // empty pending and do overlaping sync again
            info!("Need to do overlaping sync again");
            self.pending.clear();
        }

        // 5. Continue to active listen on eth events. and prune(commit to db) dequeue for older finalized events
        while self.pending.len() > self.config.fuel_finality_slider() as usize {
            self.apply_last_validator_diff(best_block.as_u64()).await;
        }

        if let Some(watcher) = watcher {
            Ok(watcher)
        } else {
            Err(RelayerError::ProviderError.into())
        }
    }

    // probably not going to metter a lot we expect for validator stake to be mostly unchanged.
    // TODO in becomes troublesome to load and takes a lot of time, it is good to optimize
    async fn load_current_validator_set(&mut self, best_fuel_block: u64) {
        let mut validator_set = HashMap::new();
        for (_, diffs) in self
            .db
            .get_validator_changes(0, Some(best_fuel_block))
            .await
        {
            validator_set.extend(diffs)
        }
        self.current_fuel_block = best_fuel_block;
    }

    /// Starting point of relayer
    pub async fn run<P>(self, provider: Provider<P>, best_fuel_block: u64)
    where
        P: JsonRpcClient + PubsubClient,
    {
        let mut this = self;

        // iterate over validator sets and update it to best_fuel_block.
        this.load_current_validator_set(best_fuel_block).await;

        loop {
            let mut logs_watcher = match this.inital_sync(&provider).await {
                Ok(watcher) => watcher,
                Err(err) => {
                    error!("Initial sync error:{}, try again", err);
                    continue;
                }
            };

            if this.status == RelayerStatus::Stop {
                return;
            }

            tokio::select! {
                inner_fuel_event = this.receiver.recv() => {
                    if inner_fuel_event.is_none() {
                        error!("Inner fuel notification broke and returned err");
                        this.status = RelayerStatus::Stop;
                    }
                    this.handle_inner_fuel_event(inner_fuel_event.unwrap()).await;
                }
                log = logs_watcher.next() => {
                    this.handle_eth_event(log).await
                }
            }
        }
    }

    async fn handle_inner_fuel_event(&mut self, inner_event: RelayerEvent) {
        match inner_event {
            RelayerEvent::Stop => {
                self.status = RelayerStatus::Stop;
            }
            RelayerEvent::NewBlock(fuel_block) => {
                // ignore reorganization

                let finality_slider = self.config.fuel_finality_slider();
                let validator_set_block =
                    std::cmp::max(fuel_block, finality_slider) - finality_slider;

                // TODO handle lagging here. compare current_fuel_block and finalized_fuel_block and send error notification
                // if we are lagging over ethereum events.

                // first if is for start of contract and first few validator blocks
                if validator_set_block < finality_slider {
                    if self.current_fuel_block != 0 {
                        error!("Initial sync seems incorrent. current_fuel_block should be zero but it is {}",self.current_fuel_block);
                    }
                    return;
                } else {
                    // we assume that for every new fuel block number is increments by one
                    let new_current_fuel_block = self.current_fuel_block + 1;
                    if new_current_fuel_block != validator_set_block {
                        error!("Inconsistency in new fuel block new validator set block is {} but our current is {}",validator_set_block,self.current_fuel_block);
                    }
                    let mut db_changes = self
                        .db
                        .get_validator_changes(new_current_fuel_block, Some(new_current_fuel_block))
                        .await;
                    if let Some((_, changes)) = db_changes.pop() {
                        for (address, new_value) in changes {
                            self.current_validator_set.insert(address, new_value);
                        }
                    }
                    self.current_fuel_block = new_current_fuel_block;
                }
            }
            RelayerEvent::GetValidatorSet {
                fuel_block,
                response_channel,
            } => {
                let finality_slider = self.config.fuel_finality_slider();
                let validator_set_block =
                    std::cmp::max(fuel_block, finality_slider) - finality_slider;

                let validators = if validator_set_block == self.current_fuel_block {
                    // if we are asking current validator set just return them.
                    // In first impl this is only thing we will need.
                    Ok(self.current_validator_set.clone())
                } else if validator_set_block > self.finalized_fuel_block {
                    // we are lagging over ethereum finalization
                    warn!("We started lagging over eth finalization");
                    Err(RelayerError::ProviderError)
                } else {
                    //TODO make this available for all validator sets, go over db and apply diffs between them.
                    // for first iteration it is not needed.
                    Err(RelayerError::ProviderError)
                };
                let _ = response_channel.send(validators);
            }
            RelayerEvent::GetStatus { response } => {
                let _ = response.send(self.status.clone());
            }
        }
    }

    async fn revert_eth_event(&mut self, fuel_event: &EthEventLog) {
        match *fuel_event {
            EthEventLog::AssetDeposit { deposit_nonce, .. } => {
                if let Some(pending) = self.pending.front_mut() {
                    pending.assets_deposited.remove(&deposit_nonce);
                }
            }
            EthEventLog::ValidatorDeposit { depositor, deposit } => {
                // okay to ignore, it is initial sync
                if let Some(pending) = self.pending.front_mut() {
                    // TODO check casting between i64 and u64
                    *pending.stake_diff.entry(depositor).or_insert(0) -= deposit as i64;
                }
            }
            EthEventLog::ValidatorWithdrawal {
                withdrawer,
                withdrawal,
            } => {
                // okay to ignore, it is initial sync
                if let Some(pending) = self.pending.front_mut() {
                    *pending.stake_diff.entry(withdrawer).or_insert(0) += withdrawal as i64;
                }
            }
            EthEventLog::FuelBlockCommited { .. } => {
                //fuel block commit reverted, just pop from pending deque
                if !self.pending.is_empty() {
                    self.pending.pop_front();
                }
                if let Some(parent) = self.pending.front_mut() {
                    parent.eth_number = u64::MAX;
                }
            }
        }
    }

    /// At begining we will ignore all event until event for new fuel block commit commes
    /// after that syncronization can start.
    async fn append_eth_events(&mut self, fuel_event: &EthEventLog, eth_block_number: u64) {
        match *fuel_event {
            EthEventLog::AssetDeposit {
                account,
                token,
                amount,
                deposit_nonce,
                ..
            } => {
                // what to do with deposit_nonce and block_number?
                if let Some(pending) = self.pending.front_mut() {
                    pending
                        .assets_deposited
                        .insert(deposit_nonce, (account, token, amount));
                }
            }
            EthEventLog::ValidatorDeposit { depositor, deposit } => {
                // okay to ignore, it is initial sync
                if let Some(pending) = self.pending.front_mut() {
                    // overflow is not possible
                    *pending.stake_diff.entry(depositor).or_insert(0) += deposit as i64;
                }
            }
            EthEventLog::ValidatorWithdrawal {
                withdrawer,
                withdrawal,
            } => {
                // okay to ignore, it is initial sync
                if let Some(pending) = self.pending.front_mut() {
                    // underflow should not be possible and it should be restrained by contract
                    *pending.stake_diff.entry(withdrawer).or_insert(0) -= withdrawal as i64;
                }
            }
            EthEventLog::FuelBlockCommited { height, .. } => {
                if let Some(parent) = self.pending.front_mut() {
                    parent.eth_number = eth_block_number;
                }
                self.pending.push_front(PendingDiff::new(height));
            }
        }
    }

    async fn handle_eth_event(&mut self, eth_event: Option<Log>) {
        // new log
        if eth_event.is_none() {
            // TODO make proper reconnect options.
            warn!("We broke something. Set state to not eth not connected and do retry");
            return;
        }
        let eth_event = eth_event.unwrap();
        trace!(target:"relayer", "got new log:{:?}", eth_event.block_hash);
        let fuel_event = EthEventLog::try_from(&eth_event);
        if let Err(err) = fuel_event {
            // not formated event from contract
            warn!(target:"relayer", "Eth Event not formated properly:{}",err);
            return;
        }
        let fuel_event = fuel_event.unwrap();
        // check if this is event from reorg block. if it is we just save it for later processing.
        // and only after all removed logs are received we apply them.
        if let Some(true) = eth_event.removed {
            // agregate all removed events before reverting them.
            if let Some(eth_block) = eth_event.block_number {
                // check if we have pending block for removal
                if let Some((last_eth_block, list)) = self.pending_removed_eth_events.last_mut() {
                    // check if last pending block is same as log event that we received.
                    if *last_eth_block == eth_block.as_u64() {
                        // just push it
                        list.push(fuel_event)
                    } else {
                        // if block number differs just push new block.
                        self.pending_removed_eth_events
                            .push((eth_block.as_u64(), vec![fuel_event]));
                    }
                } else {
                    // if there are not pending block for removal just add it.
                    self.pending_removed_eth_events
                        .push((eth_block.as_u64(), vec![fuel_event]));
                }
            } else {
                error!("Block number not found in eth log");
            }
            return;
        }
        // apply all reverted event
        if !self.pending_removed_eth_events.is_empty() {
            info!(target:"relayer", "Reorg happened on ethereum. Reverting {} logs",self.pending_removed_eth_events.len());

            // if there is new log that is not removed it means we can revert our pending removed eth events.
            for (_, block_events) in
                std::mem::take(&mut self.pending_removed_eth_events).into_iter()
            {
                for fuel_event in block_events.into_iter().rev() {
                    self.revert_eth_event(&fuel_event).await;
                }
            }
        }

        // apply new event to pending queue
        self.append_eth_events(&fuel_event, eth_event.block_number.unwrap().as_u64())
            .await;
    }
}
