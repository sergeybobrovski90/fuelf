use crate::client::{
    schema,
    types::{
        block::{
            Genesis,
            Header,
            PoAConsensus,
        },
        primitives::{
            Bytes32,
            Bytes64,
        },
        scalars::TransactionId,
        Block,
        Consensus,
    },
};

impl From<schema::block::Header> for Header {
    fn from(value: schema::block::Header) -> Self {
        let id: Bytes32 = value.id.0 .0.into();
        let transactions_root: Bytes32 = value.transactions_root.0 .0.into();
        let message_receipt_root: Bytes32 = value.message_receipt_root.0 .0.into();
        let prev_root: Bytes32 = value.prev_root.0 .0.into();
        let application_hash: Bytes32 = value.application_hash.0 .0.into();

        Self {
            id: id.into(),
            da_height: value.da_height.0.into(),
            transactions_count: value.transactions_count.0,
            message_receipt_count: value.message_receipt_count.0,
            transactions_root: transactions_root.into(),
            message_receipt_root: message_receipt_root.into(),
            height: value.height.0.into(),
            prev_root: prev_root.into(),
            time: value.time.0.into(),
            application_hash,
        }
    }
}

impl From<schema::block::Consensus> for Consensus {
    fn from(value: schema::block::Consensus) -> Self {
        match value {
            schema::block::Consensus::Genesis(genesis) => {
                Consensus::Genesis(genesis.into())
            }
            schema::block::Consensus::PoAConsensus(poa) => {
                Consensus::PoAConsensus(poa.into())
            }
            schema::block::Consensus::Unknown => Consensus::Unknown,
        }
    }
}

impl From<schema::block::Genesis> for Genesis {
    fn from(value: schema::block::Genesis) -> Self {
        let chain_config_hash: Bytes32 = value.chain_config_hash.0 .0.into();
        let coins_root: Bytes32 = value.coins_root.0 .0.into();
        let contracts_root: Bytes32 = value.coins_root.0 .0.into();
        let messages_root: Bytes32 = value.coins_root.0 .0.into();
        Self {
            chain_config_hash,
            coins_root: coins_root.into(),
            contracts_root: contracts_root.into(),
            messages_root: messages_root.into(),
        }
    }
}

impl From<schema::block::PoAConsensus> for PoAConsensus {
    fn from(value: schema::block::PoAConsensus) -> Self {
        let signature: Bytes64 = value.signature.0 .0.into();
        Self {
            signature: signature.into(),
        }
    }
}

impl From<schema::block::Block> for Block {
    fn from(value: schema::block::Block) -> Self {
        let id: Bytes32 = value.id.0 .0.into();
        let transactions = value
            .transactions
            .iter()
            .map(|tx| tx.id.0 .0)
            .map(Into::<Bytes32>::into)
            .map(Into::into)
            .collect::<Vec<TransactionId>>();
        let block_producer = value.block_producer().map(|key| {
            let bytes: Bytes64 = key.into();
            bytes.into()
        });
        Self {
            id: id.into(),
            header: value.header.into(),
            consensus: value.consensus.into(),
            transactions,
            block_producer,
        }
    }
}
