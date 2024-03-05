use crate::client::schema;

#[derive(Clone, Debug)]
pub struct NodeInfo {
    pub utxo_validation: bool,
    pub vm_backtrace: bool,
    pub min_gas_price: u64,
    pub max_tx: u64,
    pub max_depth: u64,
    pub node_version: String,
}

// GraphQL Translation

impl From<schema::node_info::NodeInfo> for NodeInfo {
    fn from(value: schema::node_info::NodeInfo) -> Self {
        Self {
            utxo_validation: value.utxo_validation,
            vm_backtrace: value.vm_backtrace,
            min_gas_price: value.min_gas_price.into(),
            max_tx: value.max_tx.into(),
            max_depth: value.max_depth.into(),
            node_version: value.node_version,
        }
    }
}
