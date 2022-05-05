mod block;
mod block_height;
mod coin;

pub use block::{FuelBlock, FuelBlockDb, FuelBlockHeader};
pub use block_height::BlockHeight;
pub use coin::{Coin, CoinStatus};

pub type Stake = u64;
pub type DaHeight = u64;
