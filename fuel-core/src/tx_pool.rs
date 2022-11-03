use chrono::{
    DateTime,
    Utc,
};
use fuel_core_interfaces::{
    common::fuel_vm::prelude::ProgramState,
    model::BlockId,
};
use serde::{
    Deserialize,
    Serialize,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransactionStatus {
    Submitted {
        time: DateTime<Utc>,
    },
    Success {
        block_id: BlockId,
        time: DateTime<Utc>,
        result: Option<ProgramState>,
    },
    SqueezedOut {
        reason: String,
    },
    Failed {
        block_id: BlockId,
        time: DateTime<Utc>,
        reason: String,
        result: Option<ProgramState>,
    },
}
