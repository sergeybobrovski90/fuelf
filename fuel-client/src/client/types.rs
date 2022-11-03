use crate::client::schema::{
    tx::{
        OpaqueTransaction,
        TransactionStatus as SchemaTxStatus,
    },
    ConversionError,
};
use chrono::{
    DateTime,
    Utc,
};
use fuel_vm::{
    fuel_tx::Transaction,
    fuel_types::bytes::Deserializable,
    prelude::ProgramState,
};
use serde::{
    Deserialize,
    Serialize,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResponse {
    pub transaction: Transaction,
    pub status: TransactionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionStatus {
    Submitted {
        submitted_at: DateTime<Utc>,
    },
    Success {
        block_id: String,
        time: DateTime<Utc>,
        program_state: Option<ProgramState>,
    },
    SqueezedOut {
        reason: String,
    },
    Failure {
        block_id: String,
        time: DateTime<Utc>,
        reason: String,
        program_state: Option<ProgramState>,
    },
}

impl TryFrom<SchemaTxStatus> for TransactionStatus {
    type Error = ConversionError;

    fn try_from(status: SchemaTxStatus) -> Result<Self, Self::Error> {
        Ok(match status {
            SchemaTxStatus::SubmittedStatus(s) => TransactionStatus::Submitted {
                submitted_at: s.time,
            },
            SchemaTxStatus::SuccessStatus(s) => TransactionStatus::Success {
                block_id: s.block.id.0.to_string(),
                time: s.time,
                program_state: s.program_state.map(TryInto::try_into).transpose()?,
            },
            SchemaTxStatus::FailureStatus(s) => TransactionStatus::Failure {
                block_id: s.block.id.0.to_string(),
                time: s.time,
                reason: s.reason,
                program_state: s.program_state.map(TryInto::try_into).transpose()?,
            },
            SchemaTxStatus::SqueezedOutStatus(s) => {
                TransactionStatus::SqueezedOut { reason: s.reason }
            }
        })
    }
}

impl TryFrom<OpaqueTransaction> for TransactionResponse {
    type Error = ConversionError;

    fn try_from(value: OpaqueTransaction) -> Result<Self, Self::Error> {
        let bytes = value.raw_payload.0 .0;
        let tx: ::fuel_vm::fuel_tx::Transaction =
            ::fuel_vm::fuel_tx::Transaction::from_bytes(bytes.as_slice())
                .map_err(ConversionError::TransactionFromBytesError)?;
        let status = value
            .status
            .ok_or_else(|| ConversionError::MissingField("status".to_string()))?
            .try_into()?;

        Ok(Self {
            transaction: tx,
            status,
        })
    }
}
