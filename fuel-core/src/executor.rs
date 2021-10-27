use crate::{
    database::{Database, DatabaseTrait, KvStoreError},
    model::{
        coin::{Coin, CoinStatus, TxoPointer},
        fuel_block::{BlockHeight, FuelBlock},
    },
    tx_pool::TransactionStatus,
};
use fuel_asm::Word;
use fuel_storage::Storage;
use fuel_tx::{Address, Bytes32, Color, Input, Output, Receipt, Transaction};
use fuel_vm::{interpreter::ExecuteError, prelude::Interpreter};
use std::error::Error as StdError;
use std::ops::DerefMut;
use thiserror::Error;

pub struct Executor {
    pub(crate) database: Database,
}

impl Executor {
    pub async fn execute(&self, block: &FuelBlock) -> Result<(), Error> {
        let mut block_tx = self.database.transaction();
        let block_id = block.id();
        Storage::<Bytes32, FuelBlock>::insert(block_tx.deref_mut(), &block_id, block)?;

        for (tx_index, tx_id) in block.transactions.iter().enumerate() {
            let mut sub_tx = block_tx.transaction();
            let db = sub_tx.deref_mut();
            let tx = Storage::<Bytes32, Transaction>::get(db, tx_id)?
                .ok_or(Error::MissingTransactionData {
                    block_id,
                    transaction_id: *tx_id,
                })?
                .into_owned();

            // execute vm
            let mut vm = Interpreter::with_storage(db.clone());
            let execution_result = vm.transact(tx);

            match execution_result {
                Ok(result) => {
                    // persist any outputs
                    self.persist_outputs(block.fuel_height, tx_index as u32, result.tx(), db)?;

                    // persist receipts
                    self.persist_receipts(tx_id, result.receipts(), db)?;

                    // persist tx status
                    db.update_tx_status(
                        tx_id,
                        TransactionStatus::Success {
                            block_id,
                            time: block.time,
                            result: *result.state(),
                        },
                    )?;

                    // only commit state changes if execution was a success
                    sub_tx.commit()?;
                }
                // save error status on block_tx since the sub_tx changes are dropped
                Err(e) => {
                    block_tx.update_tx_status(
                        tx_id,
                        TransactionStatus::Failed {
                            block_id,
                            time: block.time,
                            reason: e.to_string(),
                        },
                    )?;
                }
            }
        }

        block_tx.commit()?;
        Ok(())
    }

    // Waiting until accounts and genesis block setup is working
    fn _verify_input_state(
        &self,
        transaction: Transaction,
        block: FuelBlock,
    ) -> Result<(), TransactionValidityError> {
        let db = &self.database;
        for input in transaction.inputs() {
            match input {
                Input::Coin { utxo_id, .. } => {
                    if let Some(coin) = Storage::<Bytes32, Coin>::get(db, &utxo_id.clone())? {
                        if coin.status == CoinStatus::Spent {
                            return Err(TransactionValidityError::CoinAlreadySpent);
                        }
                        if block.fuel_height < coin.block_created + coin.maturity {
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

    fn persist_outputs(
        &self,
        block_height: BlockHeight,
        tx_index: u32,
        tx: &Transaction,
        db: &mut Database,
    ) -> Result<(), Error> {
        for (out_idx, output) in tx.outputs().iter().enumerate() {
            match output {
                Output::Coin { amount, color, to } => Executor::insert_coin(
                    block_height.into(),
                    tx_index,
                    out_idx as u8,
                    amount,
                    color,
                    to,
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
                    tx_index,
                    out_idx as u8,
                    amount,
                    color,
                    to,
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
        tx_index: u32,
        out_index: u8,
        amount: &Word,
        color: &Color,
        to: &Address,
        db: &mut Database,
    ) -> Result<(), Error> {
        let txo_pointer = TxoPointer {
            block_height: fuel_height,
            tx_index,
            output_index: out_index,
        };
        let coin = Coin {
            owner: *to,
            amount: *amount,
            color: *color,
            maturity: 0u32.into(),
            status: CoinStatus::Unspent,
            block_created: fuel_height.into(),
        };

        if Storage::<Bytes32, Coin>::insert(db, &txo_pointer.into(), &coin)?.is_some() {
            return Err(Error::OutputAlreadyExists);
        }
        Ok(())
    }

    pub fn persist_receipts(
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
}

#[derive(Debug, Error)]
pub enum TransactionValidityError {
    #[allow(dead_code)]
    #[error("Coin input was already spent")]
    CoinAlreadySpent,
    #[allow(dead_code)]
    #[error("Coin has not yet reached maturity")]
    CoinHasNotMatured,
    #[allow(dead_code)]
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
    #[error("output already exists")]
    OutputAlreadyExists,
    #[error("corrupted block state")]
    CorruptedBlockState(Box<dyn StdError>),
    #[error("missing transaction data for tx {transaction_id:?} in block {block_id:?}")]
    MissingTransactionData {
        block_id: Bytes32,
        transaction_id: Bytes32,
    },
    #[error("VM execution error: {0:?}")]
    VmExecution(fuel_vm::interpreter::ExecuteError),
}

impl From<crate::database::KvStoreError> for Error {
    fn from(e: KvStoreError) -> Self {
        Error::CorruptedBlockState(Box::new(e))
    }
}

impl From<fuel_vm::interpreter::ExecuteError> for Error {
    fn from(e: ExecuteError) -> Self {
        Error::VmExecution(e)
    }
}

impl From<crate::state::Error> for Error {
    fn from(e: crate::state::Error) -> Self {
        Error::CorruptedBlockState(Box::new(e))
    }
}
