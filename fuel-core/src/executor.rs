use crate::model::fuel_block::TransactionCommitment;
use crate::{
    database::{transaction::TransactionIndex, Database, KvStoreError},
    model::{
        coin::{Coin, CoinStatus},
        fuel_block::{BlockHeight, FuelBlockFull, FuelBlockLight},
    },
    service::Config,
    tx_pool::TransactionStatus,
};
use fuel_asm::Word;
use fuel_storage::Storage;
use fuel_tx::crypto::Hasher;
use fuel_tx::{Address, Bytes32, Color, Input, Output, Receipt, Transaction, UtxoId};
use fuel_vm::prelude::{Backtrace, Interpreter};
use fuel_vm::{consts::REG_SP, prelude::Backtrace as FuelBacktrace};
use itertools::Itertools;
use std::error::Error as StdError;
use std::ops::{Deref, DerefMut};
use thiserror::Error;
use tracing::warn;

///! The executor is used for block production and validation. Given a block, it will execute all
/// the transactions contained in the block and persist changes to the underlying database as needed.
/// In production mode, block fields like transaction commitments are set based on the executed txs.
/// In validation mode, the processed block commitments are compared with the proposed block.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMode {
    Production,
    Validation,
}

pub struct Executor {
    pub database: Database,
    pub config: Config,
}

impl Executor {
    pub async fn execute(
        &self,
        block: &mut FuelBlockFull,
        mode: ExecutionMode,
    ) -> Result<(), Error> {
        let block_id = block.id();
        let mut block_db_transaction = self.database.transaction();

        let mut commitment = TransactionCommitment::default();

        for (idx, tx) in block.transactions.iter_mut().enumerate() {
            let tx_id = tx.id();

            // Throw a clear error if the transaction id is a duplicate
            if Storage::<Bytes32, Transaction>::contains_key(
                block_db_transaction.deref_mut(),
                &tx_id,
            )? {
                return Err(Error::TransactionIdCollision(tx_id));
            }

            if self.config.utxo_validation {
                self.verify_input_state(
                    block_db_transaction.deref(),
                    tx,
                    block.headers.fuel_height,
                )?;
            }

            // verify that the tx has enough gas to cover committed costs
            self.verify_gas(tx)?;

            // index owners of inputs and outputs with tx-id, regardless of validity (hence block_tx instead of tx_db)
            self.persist_owners_index(
                block.headers.fuel_height,
                &tx,
                &tx_id,
                idx,
                block_db_transaction.deref_mut(),
            )?;

            // execute transaction
            // setup database view that only lives for the duration of vm execution
            let mut sub_block_db_commit = block_db_transaction.transaction();
            let sub_db_view = sub_block_db_commit.deref_mut();
            // execution vm
            let mut vm = Interpreter::with_storage(sub_db_view.clone());
            let vm_result = vm
                .transact(tx.clone())
                .map_err(|error| Error::VmExecution {
                    error,
                    transaction_id: tx_id,
                })?
                .into_owned();

            // only commit state changes if execution was a success
            if !vm_result.should_revert() {
                sub_block_db_commit.commit()?;
            }

            // update block commitment
            let tx_fee = self.total_fee_paid(tx, vm_result.receipts())?;
            // TODO: use SMT instead of this manual approach
            commitment.sum = commitment
                .sum
                .checked_add(tx_fee)
                .ok_or(Error::FeeOverflow)?;
            commitment.root = Hasher::hash(
                &commitment
                    .root
                    .as_ref()
                    .iter()
                    .chain(tx_id.as_ref().iter())
                    .copied()
                    .collect_vec(),
            );

            match mode {
                ExecutionMode::Validation => {
                    // ensure tx matches vm output exactly
                    if vm_result.tx() != tx {
                        return Err(Error::InvalidTransactionOutcome {
                            transaction_id: tx_id,
                        });
                    }
                }
                ExecutionMode::Production => {
                    // malleate the block with the resultant tx from the vm
                    *tx = vm_result.tx().clone()
                }
            }

            // Store tx into the block db transaction
            Storage::<Bytes32, Transaction>::insert(
                block_db_transaction.deref_mut(),
                &tx_id,
                vm_result.tx(),
            )?;

            // persist any outputs
            self.persist_outputs(
                block.headers.fuel_height,
                vm_result.tx(),
                block_db_transaction.deref_mut(),
            )?;

            // persist receipts
            self.persist_receipts(
                &tx_id,
                vm_result.receipts(),
                block_db_transaction.deref_mut(),
            )?;

            let status = if vm_result.should_revert() {
                self.log_backtrace(&vm, vm_result.receipts());
                // if script result exists, log reason
                if let Some((script_result, _)) = vm_result.receipts().iter().find_map(|r| {
                    if let Receipt::ScriptResult { result, gas_used } = r {
                        Some((result, gas_used))
                    } else {
                        None
                    }
                }) {
                    TransactionStatus::Failed {
                        block_id,
                        time: block.headers.time,
                        reason: format!("{:?}", script_result.reason()),
                        result: Some(*vm_result.state()),
                    }
                }
                // otherwise just log the revert arg
                else {
                    TransactionStatus::Failed {
                        block_id,
                        time: block.headers.time,
                        reason: format!("{:?}", vm_result.state()),
                        result: Some(*vm_result.state()),
                    }
                }
            } else {
                // else tx was a success
                TransactionStatus::Success {
                    block_id,
                    time: block.headers.time,
                    result: *vm_result.state(),
                }
            };

            // persist tx status at the block level
            block_db_transaction.update_tx_status(&tx_id, status)?;
        }

        // check or set transaction commitment
        match mode {
            ExecutionMode::Production => {
                block.headers.transactions_commitment = commitment;
            }
            ExecutionMode::Validation => {
                if block.headers.transactions_commitment != commitment {
                    return Err(Error::InvalidBlockCommitment);
                }
            }
        }

        // insert block into database
        Storage::<Bytes32, FuelBlockLight>::insert(
            block_db_transaction.deref_mut(),
            &block_id,
            &block.as_light(),
        )?;
        block_db_transaction.commit()?;
        Ok(())
    }

    // Waiting until accounts and genesis block setup is working
    fn verify_input_state(
        &self,
        db: &Database,
        transaction: &Transaction,
        block_height: BlockHeight,
    ) -> Result<(), TransactionValidityError> {
        for input in transaction.inputs() {
            match input {
                Input::Coin { utxo_id, .. } => {
                    if let Some(coin) = Storage::<UtxoId, Coin>::get(db, &utxo_id)? {
                        if coin.status == CoinStatus::Spent {
                            return Err(TransactionValidityError::CoinAlreadySpent);
                        }
                        if block_height < coin.block_created + coin.maturity {
                            return Err(TransactionValidityError::CoinHasNotMatured);
                        }
                    } else {
                        return Err(TransactionValidityError::CoinDoesntExist);
                    }
                }
                Input::Contract { .. } => {}
            }
        }

        Ok(())
    }

    /// Mark inputs as spent
    fn spend_inputs(&self, tx: &Transaction, db: &Database) -> Result<(), Error> {
        Ok(())
    }

    /// verify that the transaction has enough gas to cover fees
    fn verify_gas(&self, tx: &Transaction) -> Result<(), Error> {
        if tx.gas_price() != 0 || tx.byte_price() != 0 {
            let gas: Word = tx
                .inputs()
                .iter()
                .filter_map(|input| {
                    if let Input::Coin { amount, .. } = input {
                        Some(*amount)
                    } else {
                        None
                    }
                })
                .sum();
            let spent_gas: Word = tx
                .outputs()
                .iter()
                .filter_map(|output| match output {
                    Output::Coin { amount, color, .. } if color == &Color::default() => {
                        Some(amount)
                    }
                    Output::Withdrawal { amount, color, .. } if color == &Color::default() => {
                        Some(amount)
                    }
                    _ => None,
                })
                .sum();
            let byte_fees = tx.metered_bytes_size() as Word * tx.byte_price();
            let gas_fees = tx.gas_limit() * tx.gas_price();
            let total_gas_required = spent_gas
                .checked_add(byte_fees)
                .ok_or(Error::FeeOverflow)?
                .checked_add(gas_fees)
                .ok_or(Error::FeeOverflow)?;
            gas.checked_sub(total_gas_required)
                .ok_or(Error::InsufficientGas {
                    provided: gas,
                    required: total_gas_required,
                })?;
        }

        Ok(())
    }

    fn total_fee_paid(&self, tx: &Transaction, receipts: &[Receipt]) -> Result<Word, Error> {
        let mut fee = tx.metered_bytes_size() as Word * tx.byte_price();

        for r in receipts {
            match r {
                Receipt::ScriptResult { gas_used, .. } => {
                    fee = fee.checked_add(*gas_used).ok_or(Error::FeeOverflow)?;
                }
                _ => {}
            }
        }

        Ok(fee)
    }

    /// Log a VM backtrace if configured to do so
    fn log_backtrace(&self, vm: &Interpreter<Database>, receipts: &[Receipt]) {
        if self.config.vm.backtrace {
            if let Some(backtrace) = receipts
                .iter()
                .find_map(Receipt::result)
                .copied()
                .map(|result| Backtrace::from_vm_error(vm, result))
            {
                warn!(
                    target = "vm",
                    "Backtrace on contract: 0x{:x}\nregisters: {:?}\ncall_stack: {:?}\nstack\n: {}",
                    backtrace.contract(),
                    backtrace.registers(),
                    backtrace.call_stack(),
                    hex::encode(&backtrace.memory()[..backtrace.registers()[REG_SP] as usize]), // print stack
                );
            }
        }
    }

    fn persist_outputs(
        &self,
        block_height: BlockHeight,
        tx: &Transaction,
        db: &mut Database,
    ) -> Result<(), Error> {
        let id = tx.id();
        for (out_idx, output) in tx.outputs().iter().enumerate() {
            match output {
                Output::Coin { amount, color, to } => Executor::insert_coin(
                    block_height.into(),
                    id,
                    out_idx as u8,
                    &amount,
                    &color,
                    &to,
                    db,
                )?,
                Output::Contract {
                    balance_root: _,
                    input_index: _,
                    state_root: _,
                } => {}
                Output::Withdrawal { .. } => {}
                Output::Change { to, color, amount } => Executor::insert_coin(
                    block_height.into(),
                    id,
                    out_idx as u8,
                    &amount,
                    &color,
                    &to,
                    db,
                )?,
                Output::Variable { .. } => {}
                Output::ContractCreated { .. } => {}
            }
        }
        Ok(())
    }

    fn insert_coin(
        fuel_height: u32,
        tx_id: Bytes32,
        output_index: u8,
        amount: &Word,
        color: &Color,
        to: &Address,
        db: &mut Database,
    ) -> Result<(), Error> {
        let utxo_id = UtxoId::new(tx_id, output_index);
        let coin = Coin {
            owner: *to,
            amount: *amount,
            color: *color,
            maturity: 0u32.into(),
            status: CoinStatus::Unspent,
            block_created: fuel_height.into(),
        };

        if Storage::<UtxoId, Coin>::insert(db, &utxo_id, &coin)?.is_some() {
            return Err(Error::OutputAlreadyExists);
        }
        Ok(())
    }

    fn persist_receipts(
        &self,
        tx_id: &Bytes32,
        receipts: &[Receipt],
        db: &mut Database,
    ) -> Result<(), Error> {
        if Storage::<Bytes32, Vec<Receipt>>::insert(db, tx_id, &Vec::from(receipts))?.is_some() {
            return Err(Error::OutputAlreadyExists);
        }
        Ok(())
    }

    /// Index the tx id by owner for all of the inputs and outputs
    fn persist_owners_index(
        &self,
        block_height: BlockHeight,
        tx: &Transaction,
        tx_id: &Bytes32,
        tx_idx: usize,
        db: &mut Database,
    ) -> Result<(), Error> {
        let mut owners = vec![];
        for input in tx.inputs() {
            if let Input::Coin { owner, .. } = input {
                owners.push(owner);
            }
        }

        for output in tx.outputs() {
            match output {
                Output::Coin { to, .. }
                | Output::Withdrawal { to, .. }
                | Output::Change { to, .. }
                | Output::Variable { to, .. } => {
                    owners.push(to);
                }
                Output::Contract { .. } | Output::ContractCreated { .. } => {}
            }
        }

        // dedupe owners from inputs and outputs prior to indexing
        owners.sort();
        owners.dedup();

        for owner in owners {
            db.record_tx_id_owner(&owner, block_height, tx_idx as TransactionIndex, tx_id)?;
        }

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum TransactionValidityError {
    #[error("Coin input was already spent")]
    CoinAlreadySpent,
    #[error("Coin has not yet reached maturity")]
    CoinHasNotMatured,
    #[error("The specified coin doesn't exist")]
    CoinDoesntExist,
    #[error("Datastore error occurred")]
    DataStoreError(Box<dyn std::error::Error>),
}

impl From<crate::database::KvStoreError> for TransactionValidityError {
    fn from(e: crate::database::KvStoreError) -> Self {
        Self::DataStoreError(Box::new(e))
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Transaction id was already used: {0:#x}")]
    TransactionIdCollision(Bytes32),
    #[error("output already exists")]
    OutputAlreadyExists,
    #[error("Transaction doesn't include enough value to pay for gas: {provided} < {required}")]
    InsufficientGas { provided: Word, required: Word },
    #[error("The computed fee caused an integer overflow")]
    FeeOverflow,
    #[error("Invalid transaction: {0}")]
    TransactionValidity(#[from] TransactionValidityError),
    #[error("corrupted block state")]
    CorruptedBlockState(Box<dyn StdError>),
    #[error("missing transaction data for tx {transaction_id:#x} in block {block_id:#x}")]
    MissingTransactionData {
        block_id: Bytes32,
        transaction_id: Bytes32,
    },
    #[error("Transaction({transaction_id:#x}) execution error: {error:?}")]
    VmExecution {
        error: fuel_vm::prelude::InterpreterError,
        transaction_id: Bytes32,
    },
    #[error("Execution error with backtrace")]
    Backtrace(Box<FuelBacktrace>),
    #[error("Transaction doesn't match expected result: {transaction_id:#x}")]
    InvalidTransactionOutcome { transaction_id: Bytes32 },
    #[error("Block commitment data is invalid")]
    InvalidBlockCommitment,
}

impl From<FuelBacktrace> for Error {
    fn from(e: FuelBacktrace) -> Self {
        Error::Backtrace(Box::new(e))
    }
}

impl From<crate::database::KvStoreError> for Error {
    fn from(e: KvStoreError) -> Self {
        Error::CorruptedBlockState(Box::new(e))
    }
}

impl From<crate::state::Error> for Error {
    fn from(e: crate::state::Error) -> Self {
        Error::CorruptedBlockState(Box::new(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuel_vm::util::test_helpers::TestBuilder as TxBuilder;
    use itertools::Itertools;
    use rand::prelude::StdRng;
    use rand::{Rng, SeedableRng};

    fn test_block(num_txs: usize) -> FuelBlockFull {
        let transactions = (1..num_txs + 1)
            .into_iter()
            .map(|i| {
                TxBuilder::new(2322u64)
                    .gas_limit(10)
                    .coin_input(Color::default(), (i as Word) * 100)
                    .coin_output(Color::default(), (i as Word) * 50)
                    .change_output(Color::default())
                    .build()
            })
            .collect_vec();

        FuelBlockFull {
            headers: Default::default(),
            transactions,
        }
    }

    // Happy path test case that a produced block will also validate
    #[tokio::test]
    async fn executor_validates_correctly_produced_block() {
        let producer = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };
        let verifier = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };
        let mut block = test_block(10);

        producer
            .execute(&mut block, ExecutionMode::Production)
            .await
            .unwrap();

        let validation_result = verifier
            .execute(&mut block, ExecutionMode::Validation)
            .await;
        assert!(validation_result.is_ok());
    }

    // Ensure transaction commitment != default after execution
    #[tokio::test]
    async fn executor_commits_transactions_to_block() {
        let producer = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };
        let mut block = test_block(10);
        let start_block = block.clone();

        producer
            .execute(&mut block, ExecutionMode::Production)
            .await
            .unwrap();

        assert_ne!(
            start_block.headers.transactions_commitment,
            block.headers.transactions_commitment
        )
    }

    // Ensure tx has at least one input to cover gas
    #[tokio::test]
    async fn executor_invalidates_missing_gas_input() {
        let producer = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };

        let verifier = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };

        let gas_limit = 100;
        let gas_price = 1;
        let mut tx = Transaction::default();
        tx.set_gas_limit(gas_limit);
        tx.set_gas_price(gas_price);

        let mut block = FuelBlockFull {
            headers: Default::default(),
            transactions: vec![tx],
        };

        let produce_result = producer
            .execute(&mut block, ExecutionMode::Production)
            .await;
        assert!(matches!(
            produce_result,
            Err(Error::InsufficientGas { required, .. }) if required == gas_limit
        ));

        let verify_result = verifier
            .execute(&mut block, ExecutionMode::Validation)
            .await;
        assert!(matches!(
            verify_result,
            Err(Error::InsufficientGas {required, ..}) if required == gas_limit
        ))
    }

    #[tokio::test]
    async fn executor_invalidates_duplicate_tx_id() {
        let producer = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };

        let verifier = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };

        let mut block = FuelBlockFull {
            headers: Default::default(),
            transactions: vec![Transaction::default(), Transaction::default()],
        };

        let produce_result = producer
            .execute(&mut block, ExecutionMode::Production)
            .await;
        assert!(matches!(
            produce_result,
            Err(Error::TransactionIdCollision(_))
        ));

        let verify_result = verifier
            .execute(&mut block, ExecutionMode::Validation)
            .await;
        assert!(matches!(
            verify_result,
            Err(Error::TransactionIdCollision(_))
        ));
    }

    // invalidate a block if a tx input contains a previously used txo
    #[tokio::test]
    async fn executor_invalidates_spent_inputs() {
        let mut rng = StdRng::seed_from_u64(2322u64);

        let spent_utxo_id = rng.gen();
        let owner = Default::default();
        let amount = 10;
        let color = Default::default();
        let maturity = Default::default();
        let block_created = Default::default();
        let coin = Coin {
            owner,
            amount,
            color,
            maturity,
            status: CoinStatus::Spent,
            block_created,
        };

        let mut db = Database::default();
        // initialize database with coin that was already spent
        Storage::<UtxoId, Coin>::insert(&mut db, &spent_utxo_id, &coin).unwrap();

        // create an input referring to a coin that is already spent
        let input = Input::coin(spent_utxo_id, owner, amount, color, 0, 0, vec![], vec![]);
        let output = Output::Change {
            to: owner,
            amount: 0,
            color,
        };
        let tx = Transaction::script(
            0,
            0,
            0,
            0,
            vec![],
            vec![],
            vec![input],
            vec![output],
            vec![Default::default()],
        );

        // setup executor with utxo-validation enabled
        let config = Config {
            utxo_validation: true,
            ..Config::local_node()
        };
        let producer = Executor {
            database: db.clone(),
            config: config.clone(),
        };

        let verifier = Executor {
            database: db.clone(),
            config: config.clone(),
        };

        let mut block = FuelBlockFull {
            headers: Default::default(),
            transactions: vec![tx],
        };

        let produce_result = producer
            .execute(&mut block, ExecutionMode::Production)
            .await;
        assert!(matches!(
            produce_result,
            Err(Error::TransactionValidity(
                TransactionValidityError::CoinAlreadySpent
            ))
        ));

        let verify_result = verifier
            .execute(&mut block, ExecutionMode::Validation)
            .await;
        assert!(matches!(
            verify_result,
            Err(Error::TransactionValidity(
                TransactionValidityError::CoinAlreadySpent
            ))
        ));
    }

    // invalidate a block if a tx input doesn't exist
    #[tokio::test]
    async fn executor_invalidates_missing_inputs() {
        // create an input referring to a coin that is already spent
        let tx = TxBuilder::new(2322u64)
            .gas_limit(1)
            .coin_input(Default::default(), 10)
            .change_output(Default::default())
            .build();

        // setup executors with utxo-validation enabled
        let config = Config {
            utxo_validation: true,
            ..Config::local_node()
        };
        let producer = Executor {
            database: Database::default(),
            config: config.clone(),
        };

        let verifier = Executor {
            database: Default::default(),
            config: config.clone(),
        };

        let mut block = FuelBlockFull {
            headers: Default::default(),
            transactions: vec![tx],
        };

        let produce_result = producer
            .execute(&mut block, ExecutionMode::Production)
            .await;
        assert!(matches!(
            produce_result,
            Err(Error::TransactionValidity(
                TransactionValidityError::CoinDoesntExist
            ))
        ));

        let verify_result = verifier
            .execute(&mut block, ExecutionMode::Validation)
            .await;
        assert!(matches!(
            verify_result,
            Err(Error::TransactionValidity(
                TransactionValidityError::CoinDoesntExist
            ))
        ));
    }

    // corrupt a produced block by randomizing change amount
    // and verify that the executor invalidates the tx
    #[tokio::test]
    async fn executor_invalidates_blocks_with_diverging_tx_outputs() {
        let input_amount = 10;
        let fake_output_amount = 100;

        let tx = TxBuilder::new(2322u64)
            .gas_limit(1)
            .coin_input(Default::default(), input_amount)
            .change_output(Default::default())
            .build();

        let tx_id = tx.id();

        let producer = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };

        let verifier = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };

        let mut block = FuelBlockFull {
            headers: Default::default(),
            transactions: vec![tx],
        };

        producer
            .execute(&mut block, ExecutionMode::Production)
            .await
            .unwrap();

        // modify change amount
        if let Transaction::Script { outputs, .. } = &mut block.transactions[0] {
            if let Output::Change { amount, .. } = &mut outputs[0] {
                *amount = fake_output_amount
            }
        }

        let verify_result = verifier
            .execute(&mut block, ExecutionMode::Validation)
            .await;
        assert!(matches!(
            verify_result,
            Err(Error::InvalidTransactionOutcome { transaction_id }) if transaction_id == tx_id
        ));
    }

    // corrupt the merkle sum tree commitment from a produced block and verify that the
    // validation logic will reject the block
    #[tokio::test]
    async fn executor_invalidates_blocks_with_diverging_tx_commitment() {
        let mut rng = StdRng::seed_from_u64(2322u64);
        let tx = TxBuilder::new(2322u64)
            .gas_limit(1)
            .coin_input(Default::default(), 10)
            .change_output(Default::default())
            .build();

        let producer = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };

        let verifier = Executor {
            database: Default::default(),
            config: Config::local_node(),
        };

        let mut block = FuelBlockFull {
            headers: Default::default(),
            transactions: vec![tx],
        };

        producer
            .execute(&mut block, ExecutionMode::Production)
            .await
            .unwrap();

        // randomize transaction commitment
        block.headers.transactions_commitment.root = rng.gen();
        block.headers.transactions_commitment.sum = rng.gen();

        let verify_result = verifier
            .execute(&mut block, ExecutionMode::Validation)
            .await;

        assert!(matches!(verify_result, Err(Error::InvalidBlockCommitment)))
    }

    #[tokio::test]
    async fn validation_succeeds_when_input_contract_utxo_id_uses_expected_value() {
        // create a contract in block 1
        // verify a block 2 containing contract id from block 1, using the correct contract utxo_id from block 1.
        unimplemented!()
    }

    // verify that a contract input must exist for a transaction
    #[tokio::test]
    async fn invalidates_if_input_contract_utxo_id_divergent() {
        // create a contract in block 1
        // verify a block 2 containing contract id from block 1, with wrong input contract utxo_id
        unimplemented!()
    }

    // verify that a contract output is set for a transaction
    #[tokio::test]
    async fn contract_output_is_set() {
        unimplemented!()
    }

    // If a produced block creates a different contract output than what the verifier expects,
    // invalidate the block
    #[tokio::test]
    async fn executor_invalidates_divergent_contract_outputs() {
        unimplemented!()
    }
}
