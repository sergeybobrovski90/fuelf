use super::{
    input::Input,
    output::Output,
    receipt::Receipt,
};
use crate::{
    database::{
        storage::{
            FuelBlocks,
            Receipts,
        },
        Database,
    },
    schema::{
        block::Block,
        contract::Contract,
        scalars::{
            AssetId,
            Bytes32,
            HexString,
            Salt,
            TransactionId,
            TxPointer,
            U64,
        },
    },
    tx_pool::TransactionStatus as TxStatus,
};
use async_graphql::{
    Context,
    Enum,
    Object,
    Union,
};
use chrono::{
    DateTime,
    Utc,
};
use fuel_core_interfaces::{
    common::{
        fuel_storage::StorageAsRef,
        fuel_tx,
        fuel_tx::{
            field::{
                BytecodeLength,
                BytecodeWitnessIndex,
                Inputs,
                Maturity,
                Outputs,
                ReceiptsRoot,
                Salt as SaltField,
                Script as ScriptField,
                ScriptData,
                StorageSlots,
                TxPointer as TxPointerField,
                Witnesses,
            },
            Chargeable,
            Executable,
            UniqueIdentifier,
        },
        fuel_types,
        fuel_types::bytes::SerializableVec,
        fuel_vm::prelude::ProgramState as VmProgramState,
    },
    db::KvStoreError,
    txpool::TxPoolMpsc,
};
use fuel_txpool::Service as TxPoolService;
use std::sync::Arc;
use tokio::sync::oneshot;

pub struct ProgramState {
    return_type: ReturnType,
    data: Vec<u8>,
}

#[Object]
impl ProgramState {
    async fn return_type(&self) -> ReturnType {
        self.return_type
    }

    async fn data(&self) -> HexString {
        self.data.clone().into()
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ReturnType {
    Return,
    ReturnData,
    Revert,
}

impl From<VmProgramState> for ProgramState {
    fn from(state: VmProgramState) -> Self {
        match state {
            VmProgramState::Return(d) => ProgramState {
                return_type: ReturnType::Return,
                data: d.to_be_bytes().to_vec(),
            },
            VmProgramState::ReturnData(d) => ProgramState {
                return_type: ReturnType::ReturnData,
                data: d.as_ref().to_vec(),
            },
            VmProgramState::Revert(d) => ProgramState {
                return_type: ReturnType::Revert,
                data: d.to_be_bytes().to_vec(),
            },
            #[cfg(feature = "debug")]
            VmProgramState::RunProgram(_) | VmProgramState::VerifyPredicate(_) => {
                unreachable!("This shouldn't get called with a debug state")
            }
        }
    }
}

#[derive(Union)]
pub enum TransactionStatus {
    Submitted(SubmittedStatus),
    Success(SuccessStatus),
    Failed(FailureStatus),
}

pub struct SubmittedStatus(DateTime<Utc>);

#[Object]
impl SubmittedStatus {
    async fn time(&self) -> DateTime<Utc> {
        self.0
    }
}

pub struct SuccessStatus {
    block_id: fuel_core_interfaces::model::BlockId,
    time: DateTime<Utc>,
    result: VmProgramState,
}

#[Object]
impl SuccessStatus {
    async fn block(&self, ctx: &Context<'_>) -> async_graphql::Result<Block> {
        let db = ctx.data_unchecked::<Database>();
        let block = db
            .storage::<FuelBlocks>()
            .get(&self.block_id.into())?
            .ok_or(KvStoreError::NotFound)?
            .into_owned();
        let block = Block::from(block);
        Ok(block)
    }

    async fn time(&self) -> DateTime<Utc> {
        self.time
    }

    async fn program_state(&self) -> ProgramState {
        self.result.into()
    }
}

pub struct FailureStatus {
    block_id: fuel_core_interfaces::model::BlockId,
    time: DateTime<Utc>,
    reason: String,
    state: Option<VmProgramState>,
}

#[Object]
impl FailureStatus {
    async fn block(&self, ctx: &Context<'_>) -> async_graphql::Result<Block> {
        let db = ctx.data_unchecked::<Database>();
        let block = db
            .storage::<FuelBlocks>()
            .get(&self.block_id.into())?
            .ok_or(KvStoreError::NotFound)?
            .into_owned();
        let block = Block::from(block);
        Ok(block)
    }

    async fn time(&self) -> DateTime<Utc> {
        self.time
    }

    async fn reason(&self) -> String {
        self.reason.clone()
    }

    async fn program_state(&self) -> Option<ProgramState> {
        self.state.map(Into::into)
    }
}

impl From<TxStatus> for TransactionStatus {
    fn from(s: TxStatus) -> Self {
        match s {
            TxStatus::Submitted { time } => {
                TransactionStatus::Submitted(SubmittedStatus(time))
            }
            TxStatus::Success {
                block_id,
                result,
                time,
            } => TransactionStatus::Success(SuccessStatus {
                block_id,
                result,
                time,
            }),
            TxStatus::Failed {
                block_id,
                reason,
                time,
                result,
            } => TransactionStatus::Failed(FailureStatus {
                block_id,
                reason,
                time,
                state: result,
            }),
        }
    }
}

pub struct Transaction(pub(crate) fuel_tx::Transaction);

#[Object]
impl Transaction {
    async fn id(&self) -> TransactionId {
        TransactionId(self.0.id())
    }

    async fn input_asset_ids(&self) -> Option<Vec<AssetId>> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => {
                Some(script.input_asset_ids().map(|c| AssetId(*c)).collect())
            }
            fuel_tx::Transaction::Create(create) => {
                Some(create.input_asset_ids().map(|c| AssetId(*c)).collect())
            }
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn input_contracts(&self) -> Option<Vec<Contract>> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => {
                Some(script.input_contracts().map(|v| Contract(*v)).collect())
            }
            fuel_tx::Transaction::Create(create) => {
                Some(create.input_contracts().map(|v| Contract(*v)).collect())
            }
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn gas_price(&self) -> Option<U64> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => Some(script.price().into()),
            fuel_tx::Transaction::Create(create) => Some(create.price().into()),
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn gas_limit(&self) -> Option<U64> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => Some(script.limit().into()),
            fuel_tx::Transaction::Create(create) => Some(create.limit().into()),
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn maturity(&self) -> Option<U64> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => Some((*script.maturity()).into()),
            fuel_tx::Transaction::Create(create) => Some((*create.maturity()).into()),
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn tx_pointer(&self) -> Option<TxPointer> {
        match &self.0 {
            fuel_tx::Transaction::Script(_) => None,
            fuel_tx::Transaction::Create(_) => None,
            fuel_tx::Transaction::Mint(mint) => Some((*mint.tx_pointer()).into()),
        }
    }

    async fn is_script(&self) -> bool {
        self.0.is_script()
    }

    async fn is_create(&self) -> bool {
        self.0.is_create()
    }

    async fn is_mint(&self) -> bool {
        self.0.is_mint()
    }

    async fn inputs(&self) -> Option<Vec<Input>> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => {
                Some(script.inputs().iter().map(Into::into).collect())
            }
            fuel_tx::Transaction::Create(create) => {
                Some(create.inputs().iter().map(Into::into).collect())
            }
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn outputs(&self) -> Vec<Output> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => {
                script.outputs().iter().map(Into::into).collect()
            }
            fuel_tx::Transaction::Create(create) => {
                create.outputs().iter().map(Into::into).collect()
            }
            fuel_tx::Transaction::Mint(mint) => {
                mint.outputs().iter().map(Into::into).collect()
            }
        }
    }

    async fn witnesses(&self) -> Option<Vec<HexString>> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => Some(
                script
                    .witnesses()
                    .iter()
                    .map(|w| HexString(w.clone().into_inner()))
                    .collect(),
            ),
            fuel_tx::Transaction::Create(create) => Some(
                create
                    .witnesses()
                    .iter()
                    .map(|w| HexString(w.clone().into_inner()))
                    .collect(),
            ),
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn receipts_root(&self) -> Option<Bytes32> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => {
                Some((*script.receipts_root()).into())
            }
            fuel_tx::Transaction::Create(_) => None,
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn status(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Option<TransactionStatus>> {
        let db = ctx.data_unchecked::<Database>();
        let txpool = ctx.data_unchecked::<Arc<TxPoolService>>();
        let id = self.0.id();

        let (response, receiver) = oneshot::channel();
        let _ = txpool
            .sender()
            .send(TxPoolMpsc::FindOne { id, response })
            .await;

        if let Ok(Some(transaction_in_pool)) = receiver.await {
            let time = transaction_in_pool.submitted_time();
            Ok(Some(TransactionStatus::Submitted(SubmittedStatus(time))))
        } else {
            let status = db.get_tx_status(&self.0.id())?;
            Ok(status.map(Into::into))
        }
    }

    async fn receipts(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Option<Vec<Receipt>>> {
        let db = ctx.data_unchecked::<Database>();
        let receipts = db.storage::<Receipts>().get(&self.0.id())?;
        Ok(receipts.map(|receipts| receipts.iter().cloned().map(Receipt).collect()))
    }

    async fn script(&self) -> Option<HexString> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => {
                Some(HexString(script.script().clone()))
            }
            fuel_tx::Transaction::Create(_) => None,
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn script_data(&self) -> Option<HexString> {
        match &self.0 {
            fuel_tx::Transaction::Script(script) => {
                Some(HexString(script.script_data().clone()))
            }
            fuel_tx::Transaction::Create(_) => None,
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn bytecode_witness_index(&self) -> Option<u8> {
        match &self.0 {
            fuel_tx::Transaction::Script(_) => None,
            fuel_tx::Transaction::Create(create) => {
                Some(*create.bytecode_witness_index())
            }
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn bytecode_length(&self) -> Option<U64> {
        match &self.0 {
            fuel_tx::Transaction::Script(_) => None,
            fuel_tx::Transaction::Create(create) => {
                Some((*create.bytecode_length()).into())
            }
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn salt(&self) -> Option<Salt> {
        match &self.0 {
            fuel_tx::Transaction::Script(_) => None,
            fuel_tx::Transaction::Create(create) => Some((*create.salt()).into()),
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    async fn storage_slots(&self) -> Option<Vec<HexString>> {
        match &self.0 {
            fuel_tx::Transaction::Script(_) => None,
            fuel_tx::Transaction::Create(create) => Some(
                create
                    .storage_slots()
                    .iter()
                    .map(|slot| {
                        HexString(
                            slot.key()
                                .as_slice()
                                .iter()
                                .chain(slot.value().as_slice())
                                .copied()
                                .collect(),
                        )
                    })
                    .collect(),
            ),
            fuel_tx::Transaction::Mint(_) => None,
        }
    }

    /// Return the transaction bytes using canonical encoding
    async fn raw_payload(&self) -> HexString {
        HexString(self.0.clone().to_bytes())
    }
}
