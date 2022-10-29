use lazy_static::lazy_static;
pub use prometheus_client::metrics::histogram::Histogram;
use prometheus_client::registry::Registry;
use std::{
    boxed::Box,
    default::Default,
};

pub struct TxPoolMetrics {
    pub registry: Registry,
    pub gas_price_histogram: Histogram,
    pub tx_size_histogram: Histogram,
}

impl Default for TxPoolMetrics {
    fn default() -> Self {
        let registry = Registry::default();

        let gas_prices = Vec::new();

        let gas_price_histogram = Histogram::new(gas_prices.into_iter());

        let tx_sizes = Vec::new();

        let tx_size_histogram = Histogram::new(tx_sizes.into_iter());

        Self {
            registry,
            gas_price_histogram,
            tx_size_histogram,
        }
    }
}

pub fn init(mut metrics: TxPoolMetrics) -> TxPoolMetrics {
    metrics.registry.register(
        "Tx_Gas_Price_Histogram",
        "A Histogram keeping track of all gas prices for each tx in the mempool",
        Box::new(metrics.gas_price_histogram.clone()),
    );

    metrics.registry.register(
        "Tx_Size_Histogram",
        "A Histogram keeping track of the size of txs",
        Box::new(metrics.tx_size_histogram.clone()),
    );

    metrics
}

lazy_static! {
    pub static ref TXPOOL_METRICS: TxPoolMetrics = {
        // Registries which are initialized inside the fuel-metrics crate are safe
        // since they cannot be called before the registry is intialized
        let registry = TxPoolMetrics::default();

        init(registry)
    };
}
