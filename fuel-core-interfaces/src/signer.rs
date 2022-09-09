use async_trait::async_trait;
use fuel_types::Bytes32;
use thiserror::Error;

/// Dummy signer that will be removed in next pull request.
/// TODO: Do not use.
#[async_trait]
pub trait Signer {
    async fn sign(&self, hash: &Bytes32) -> Result<Bytes32, SignerError>;
}

#[derive(Error, Debug)]
pub enum SignerError {
    #[error("Private key not loaded")]
    KeyNotLoaded,
}
