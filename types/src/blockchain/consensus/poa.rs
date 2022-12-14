use crate::fuel_crypto::Signature;

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// The consensus related data that doesn't live on the
/// header.
pub struct PoAConsensus {
    /// The signature of the [`FuelBlockHeader`].
    pub signature: Signature,
}

impl PoAConsensus {
    /// Create a new block consensus.
    pub fn new(signature: Signature) -> Self {
        Self { signature }
    }
}
