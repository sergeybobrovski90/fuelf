pub mod bounded;
pub mod postcard;
pub mod unbounded;

use crate::gossipsub::messages::GossipTopicTag;
use libp2p::request_response;
use serde::{
    Deserialize,
    Serialize,
};
use std::io;

trait DataFormatCodec {
    type Error;
    fn deserialize<'a, R: Deserialize<'a>>(
        encoded_data: &'a [u8],
    ) -> Result<R, Self::Error>;

    fn serialize<D: Serialize>(data: &D) -> Result<Vec<u8>, Self::Error>;
}

/// Implement this in order to handle serialization & deserialization of Gossipsub messages
pub trait GossipsubCodec {
    type RequestMessage;
    type ResponseMessage;

    fn encode(&self, data: Self::RequestMessage) -> Result<Vec<u8>, io::Error>;

    fn decode(
        &self,
        encoded_data: &[u8],
        gossipsub_topic: GossipTopicTag,
    ) -> Result<Self::ResponseMessage, io::Error>;
}

pub trait RequestResponseProtocols: request_response::Codec {
    /// Returns RequestResponse's Protocol
    /// Needed for initialization of RequestResponse Behaviour
    fn get_req_res_protocols(
        &self,
    ) -> impl Iterator<Item = <Self as request_response::Codec>::Protocol>;
}
