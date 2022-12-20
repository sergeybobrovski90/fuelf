#![deny(unused_crate_dependencies)]
#![deny(unused_must_use)]

pub mod block_producer;
pub mod config;
pub mod ports;

pub use block_producer::Producer;
pub use config::Config;

#[cfg(any(test, feature = "test-helpers"))]
pub mod mocks;
