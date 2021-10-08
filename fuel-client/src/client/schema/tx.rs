use crate::client::schema::{
    schema, ConnectionArgs, ConversionError, ConversionError::MissingField, HexString,
    HexString256, PageInfo,
};
use cynic::impl_scalar;
use fuel_tx::Witness;
use std::convert::{TryFrom, TryInto};

#[derive(cynic::FragmentArguments, Debug)]
pub struct TxIdArgs {
    pub id: HexString256,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Query",
    argument_struct = "TxIdArgs"
)]
pub struct TransactionQuery {
    #[arguments(id = &args.id)]
    pub transaction: Option<Transaction>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Query",
    argument_struct = "ConnectionArgs"
)]
pub struct TransactionsQuery {
    #[arguments(after = &args.after, before = &args.before, first = &args.first, last = &args.last)]
    pub transactions: TransactionConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct TransactionConnection {
    pub edges: Option<Vec<Option<TransactionEdge>>>,
    pub page_info: PageInfo,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct TransactionEdge {
    pub cursor: String,
    pub node: Transaction,
}

type Metadata = fuel_tx::Metadata;
impl_scalar!(Metadata, schema::Json);

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct Transaction {
    pub gas_limit: i32,
    pub gas_price: i32,
    pub id: HexString256,
    pub input_colors: Vec<HexString256>,
    pub input_contracts: Vec<HexString256>,
    pub inputs: Vec<Input>,
    pub is_script: bool,
    pub outputs: Vec<Output>,
    pub maturity: i32,
    pub receipts_root: Option<HexString256>,
    pub status: Option<Status>,
    pub witnesses: Vec<HexString>,
    pub receipts: Option<Vec<Receipt>>,
    pub script: Option<HexString>,
    pub script_data: Option<HexString>,
    pub metadata: Option<Metadata>,
    pub salt: Option<HexString256>,
    pub static_contracts: Option<Vec<HexString256>>,
    pub bytecode_witness_index: Option<i32>,
}

impl TryFrom<Transaction> for fuel_vm::prelude::Transaction {
    type Error = ConversionError;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        Ok(match tx.is_script {
            true => Self::Script {
                gas_price: tx.gas_price.try_into()?,
                gas_limit: tx.gas_limit.try_into()?,
                maturity: tx.maturity.try_into()?,
                receipts_root: tx
                    .receipts_root
                    .ok_or(ConversionError::MissingField("receipts_root".to_string()))?
                    .into(),
                script: tx
                    .script
                    .ok_or(ConversionError::MissingField("script".to_string()))?
                    .into(),
                script_data: tx
                    .script_data
                    .ok_or(ConversionError::MissingField("script_data".to_string()))?
                    .into(),
                inputs: tx
                    .inputs
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<fuel_tx::Input>, ConversionError>>()?,
                outputs: tx
                    .outputs
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<fuel_tx::Output>, ConversionError>>()?,
                witnesses: tx.witnesses.into_iter().map(Into::into).collect(),
                metadata: tx.metadata,
            },
            false => Self::Create {
                gas_price: tx.gas_price.try_into()?,
                gas_limit: tx.gas_limit.try_into()?,
                maturity: tx.maturity.try_into()?,
                bytecode_witness_index: tx
                    .bytecode_witness_index
                    .ok_or(ConversionError::MissingField(
                        "bytecode_witness_index".to_string(),
                    ))?
                    .try_into()?,
                salt: tx
                    .salt
                    .ok_or(ConversionError::MissingField("salt".to_string()))?
                    .into(),
                static_contracts: tx
                    .static_contracts
                    .ok_or(ConversionError::MissingField(
                        "static_contracts".to_string(),
                    ))?
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                inputs: tx
                    .inputs
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<fuel_tx::Input>, ConversionError>>()?,
                outputs: tx
                    .outputs
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<fuel_tx::Output>, ConversionError>>()?,
                witnesses: tx.witnesses.into_iter().map(Into::into).collect(),
                metadata: tx.metadata,
            },
        })
    }
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct Receipt {
    pub a: Option<i32>,
    pub b: Option<i32>,
    pub amount: Option<i32>,
    pub color: Option<HexString256>,
    pub gas: Option<i32>,
    pub digest: Option<HexString256>,
    pub id: HexString256,
    pub is: i32,
    pub pc: i32,
    pub ptr: Option<i32>,
    pub ra: Option<i32>,
    pub rb: Option<i32>,
    pub rc: Option<i32>,
    pub rd: Option<i32>,
    pub reason: Option<i32>,
    pub receipt_type: ReceiptType,
    pub to: Option<HexString256>,
    pub to_address: Option<HexString256>,
    pub val: Option<i32>,
    pub len: Option<i32>,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub enum ReceiptType {
    Call,
    Return,
    ReturnData,
    Panic,
    Revert,
    Log,
    LogData,
    Transfer,
    TransferOut,
}

impl TryFrom<Receipt> for fuel_vm::prelude::Receipt {
    type Error = ConversionError;

    fn try_from(schema: Receipt) -> Result<Self, Self::Error> {
        Ok(match schema.receipt_type {
            ReceiptType::Call => fuel_vm::prelude::Receipt::Call {
                id: schema.id.into(),
                to: schema.to.ok_or(MissingField("to".to_string()))?.into(),
                amount: schema
                    .amount
                    .ok_or(MissingField("amount".to_string()))?
                    .try_into()?,
                color: schema
                    .color
                    .ok_or(MissingField("color".to_string()))?
                    .into(),
                gas: schema
                    .gas
                    .ok_or(MissingField("gas".to_string()))?
                    .try_into()?,
                a: schema.a.ok_or(MissingField("a".to_string()))?.try_into()?,
                b: schema.b.ok_or(MissingField("b".to_string()))?.try_into()?,
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
            ReceiptType::Return => fuel_vm::prelude::Receipt::Return {
                id: schema.id.into(),
                val: schema
                    .val
                    .ok_or(MissingField("val".to_string()))?
                    .try_into()?,
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
            ReceiptType::ReturnData => fuel_vm::prelude::Receipt::ReturnData {
                id: schema.id.into(),
                ptr: schema
                    .ptr
                    .ok_or(MissingField("ptr".to_string()))?
                    .try_into()?,
                len: schema
                    .len
                    .ok_or(MissingField("len".to_string()))?
                    .try_into()?,
                digest: Default::default(),
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
            ReceiptType::Panic => fuel_vm::prelude::Receipt::Panic {
                id: schema.id.into(),
                reason: schema
                    .reason
                    .ok_or(MissingField("reason".to_string()))?
                    .try_into()?,
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
            ReceiptType::Revert => fuel_vm::prelude::Receipt::Revert {
                id: schema.id.into(),
                ra: schema
                    .ra
                    .ok_or(MissingField("ra".to_string()))?
                    .try_into()?,
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
            ReceiptType::Log => fuel_vm::prelude::Receipt::Log {
                id: schema.id.into(),
                ra: schema
                    .ra
                    .ok_or(MissingField("ra".to_string()))?
                    .try_into()?,
                rb: schema
                    .rb
                    .ok_or(MissingField("rb".to_string()))?
                    .try_into()?,
                rc: schema
                    .rc
                    .ok_or(MissingField("rc".to_string()))?
                    .try_into()?,
                rd: schema
                    .rd
                    .ok_or(MissingField("rd".to_string()))?
                    .try_into()?,
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
            ReceiptType::LogData => fuel_vm::prelude::Receipt::LogData {
                id: schema.id.into(),
                ra: schema
                    .ra
                    .ok_or(MissingField("ra".to_string()))?
                    .try_into()?,
                rb: schema
                    .rb
                    .ok_or(MissingField("rb".to_string()))?
                    .try_into()?,
                ptr: schema
                    .ptr
                    .ok_or(MissingField("ptr".to_string()))?
                    .try_into()?,
                len: schema
                    .len
                    .ok_or(MissingField("len".to_string()))?
                    .try_into()?,
                digest: schema
                    .digest
                    .ok_or(MissingField("digest".to_string()))?
                    .into(),
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
            ReceiptType::Transfer => fuel_vm::prelude::Receipt::Transfer {
                id: schema.id.into(),
                to: schema.to.ok_or(MissingField("to".to_string()))?.into(),
                amount: schema
                    .amount
                    .ok_or(MissingField("amount".to_string()))?
                    .try_into()?,
                color: schema
                    .color
                    .ok_or(MissingField("color".to_string()))?
                    .into(),
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
            ReceiptType::TransferOut => fuel_vm::prelude::Receipt::TransferOut {
                id: schema.id.into(),
                to: schema.to.ok_or(MissingField("to".to_string()))?.into(),
                amount: schema
                    .amount
                    .ok_or(MissingField("amount".to_string()))?
                    .try_into()?,
                color: schema
                    .color
                    .ok_or(MissingField("color".to_string()))?
                    .into(),
                pc: schema.pc.try_into()?,
                is: schema.is.try_into()?,
            },
        })
    }
}

impl From<HexString> for Witness {
    fn from(s: HexString) -> Self {
        let v = <Vec<u8>>::from(s);
        Self::from(v)
    }
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub enum Input {
    InputCoin(InputCoin),
    InputContract(InputContract),
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct InputCoin {
    pub utxo_id: HexString256,
    pub owner: HexString256,
    pub amount: i32,
    pub color: HexString256,
    pub witness_index: i32,
    pub maturity: i32,
    pub predicate: HexString,
    pub predicate_data: HexString,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct InputContract {
    pub utxo_id: HexString256,
    pub balance_root: HexString256,
    pub state_root: HexString256,
    pub contract_id: HexString256,
}

impl TryFrom<Input> for fuel_tx::Input {
    type Error = ConversionError;

    fn try_from(input: Input) -> Result<fuel_tx::Input, Self::Error> {
        Ok(match input {
            Input::InputCoin(coin) => fuel_tx::Input::Coin {
                utxo_id: coin.utxo_id.into(),
                owner: coin.owner.into(),
                amount: coin.amount.try_into()?,
                color: coin.color.into(),
                witness_index: coin.witness_index.try_into()?,
                maturity: coin.maturity.try_into()?,
                predicate: coin.predicate.into(),
                predicate_data: coin.predicate_data.into(),
            },
            Input::InputContract(contract) => fuel_tx::Input::Contract {
                utxo_id: contract.utxo_id.into(),
                balance_root: contract.balance_root.into(),
                state_root: contract.state_root.into(),
                contract_id: contract.contract_id.into(),
            },
        })
    }
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub enum Output {
    CoinOutput(CoinOutput),
    ContractOutput(ContractOutput),
    WithdrawalOutput(WithdrawalOutput),
    ChangeOutput(ChangeOutput),
    VariableOutput(VariableOutput),
    ContractCreated(ContractCreated),
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct CoinOutput {
    pub to: HexString256,
    pub amount: i32,
    pub color: HexString256,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct WithdrawalOutput {
    pub to: HexString256,
    pub amount: i32,
    pub color: HexString256,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct ChangeOutput {
    pub to: HexString256,
    pub amount: i32,
    pub color: HexString256,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct VariableOutput {
    pub to: HexString256,
    pub amount: i32,
    pub color: HexString256,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct ContractOutput {
    pub input_index: i32,
    pub balance_root: HexString256,
    pub state_root: HexString256,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct ContractCreated {
    contract_id: HexString256,
}

impl TryFrom<Output> for fuel_tx::Output {
    type Error = ConversionError;

    fn try_from(value: Output) -> Result<Self, Self::Error> {
        Ok(match value {
            Output::CoinOutput(coin) => Self::Coin {
                to: coin.to.into(),
                amount: coin.amount.try_into()?,
                color: coin.color.into(),
            },
            Output::ContractOutput(contract) => Self::Contract {
                input_index: contract.input_index.try_into()?,
                balance_root: contract.balance_root.into(),
                state_root: contract.state_root.into(),
            },
            Output::WithdrawalOutput(withdrawal) => Self::Withdrawal {
                to: withdrawal.to.into(),
                amount: withdrawal.amount.try_into()?,
                color: withdrawal.color.into(),
            },
            Output::ChangeOutput(change) => Self::Change {
                to: change.to.into(),
                amount: change.amount.try_into()?,
                color: change.color.into(),
            },
            Output::VariableOutput(variable) => Self::Variable {
                to: variable.to.into(),
                amount: variable.amount.try_into()?,
                color: variable.color.into(),
            },
            Output::ContractCreated(contract) => Self::ContractCreated {
                contract_id: contract.contract_id.into(),
            },
        })
    }
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub enum Status {
    SubmittedStatus(SubmittedStatus),
    SuccessStatus(SuccessStatus),
    FailureStatus(FailureStatus),
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct SubmittedStatus {
    pub time: super::DateTime,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct SuccessStatus {
    pub block_id: HexString256,
    pub program_state: HexString,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct FailureStatus {
    pub block_id: HexString256,
    pub reason: String,
}

// mutations

#[derive(cynic::FragmentArguments)]
pub struct TxArg {
    pub tx: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Mutation",
    argument_struct = "TxArg"
)]
pub struct DryRun {
    #[arguments(tx = &args.tx)]
    pub dry_run: Vec<Receipt>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_by_id_query_gql_output() {
        use cynic::QueryBuilder;
        let operation = TransactionQuery::build(TxIdArgs {
            id: HexString256::default(),
        });
        insta::assert_snapshot!(operation.query)
    }

    #[test]
    fn transactions_connection_query_gql_output() {
        use cynic::QueryBuilder;
        let operation = TransactionsQuery::build(ConnectionArgs {
            after: None,
            before: None,
            first: None,
            last: None,
        });
        insta::assert_snapshot!(operation.query)
    }
}
