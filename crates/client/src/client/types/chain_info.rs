use crate::client::{
    schema,
    types::{
        Block,
        ConsensusParameters,
    },
};

pub struct ChainInfo {
    pub base_chain_height: u32,
    pub name: String,
    pub peer_count: i32,
    pub latest_block: Block,
    pub consensus_parameters: ConsensusParameters,
}

impl From<schema::chain::ChainInfo> for ChainInfo {
    fn from(value: schema::chain::ChainInfo) -> Self {
        Self {
            base_chain_height: value.base_chain_height.into(),
            name: value.name,
            peer_count: value.peer_count.into(),
            latest_block: value.latest_block.into(),
            consensus_parameters: value.consensus_parameters.into(),
        }
    }
}
