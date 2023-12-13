use super::{
    GossipsubCodec,
    NetworkCodec,
    RequestResponseConverter,
};
use crate::{
    gossipsub::messages::{
        GossipTopicTag,
        GossipsubBroadcastRequest,
        GossipsubMessage,
    },
    request_response::messages::{
        NetworkResponse,
        OutboundResponse,
        RequestMessage,
        ResponseMessage,
        // MAX_REQUEST_SIZE,
        REQUEST_RESPONSE_PROTOCOL_ID,
    },
};
use async_trait::async_trait;
use asynchronous_codec::{
    BytesCodec,
    FramedRead,
    FramedWrite,
};
use futures::{
    AsyncRead,
    SinkExt,
    TryStreamExt,
};
use libp2p::request_response::Codec as RequestResponseCodec;
use serde::{
    Deserialize,
    Serialize,
};
use std::io;

#[derive(Default, Debug, Clone)]
pub struct PostcardCodec {
    // TODO: Do we still need this? The `BytesCodec` doesn't take a max size. Maybe we should use a different codec
    /// Used for `max_size` parameter when reading Response Message
    /// Necessary in order to avoid DoS attacks
    /// Currently the size mostly depends on the max size of the Block
    _max_response_size: usize,
}

impl PostcardCodec {
    pub fn new(max_block_size: usize) -> Self {
        Self {
            _max_response_size: max_block_size,
        }
    }

    /// Helper method for decoding data
    /// Reusable across `RequestResponseCodec` and `GossipsubCodec`
    fn deserialize<'a, R: Deserialize<'a>>(
        &self,
        encoded_data: &'a [u8],
    ) -> Result<R, io::Error> {
        postcard::from_bytes(encoded_data)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
    }

    fn serialize<D: Serialize>(&self, data: &D) -> Result<Vec<u8>, io::Error> {
        postcard::to_stdvec(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
    }
}

fn invalid_data_err() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid data in `postcard` codec",
    )
}

/// Since Postcard does not support async reads or writes out of the box
/// We prefix Request & Response Messages with the length of the data in bytes
/// We expect the substream to be properly closed when response channel is dropped.
/// Since the request protocol used here expects a response, the sender considers this
/// early close as a protocol violation which results in the connection being closed.
/// If the substream was not properly closed when dropped, the sender would instead
/// run into a timeout waiting for the response.
#[async_trait]
impl RequestResponseCodec for PostcardCodec {
    type Protocol = MessageExchangePostcardProtocol;
    type Request = RequestMessage;
    type Response = NetworkResponse;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        socket: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let encoded_data = FramedRead::new(socket, BytesCodec)
            .try_next()
            .await?
            .ok_or(invalid_data_err())?;
        self.deserialize(&encoded_data)
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        socket: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let encoded_data = FramedRead::new(socket, BytesCodec)
            .try_next()
            .await?
            .ok_or(invalid_data_err())?;

        self.deserialize(&encoded_data)
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        socket: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        match postcard::to_stdvec(&req) {
            Ok(encoded_data) => {
                let mut framed = FramedWrite::new(socket, BytesCodec);
                framed.send(encoded_data.into()).await?;
                framed.close().await?;

                Ok(())
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string())),
        }
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        socket: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        match postcard::to_stdvec(&res) {
            Ok(encoded_data) => {
                let mut framed = FramedWrite::new(socket, BytesCodec);
                framed.send(encoded_data.into()).await?;
                framed.close().await?;

                Ok(())
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string())),
        }
    }
}

impl GossipsubCodec for PostcardCodec {
    type RequestMessage = GossipsubBroadcastRequest;
    type ResponseMessage = GossipsubMessage;

    fn encode(&self, data: Self::RequestMessage) -> Result<Vec<u8>, io::Error> {
        let encoded_data = match data {
            GossipsubBroadcastRequest::NewTx(tx) => postcard::to_stdvec(&*tx),
        };

        encoded_data.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
    }

    fn decode(
        &self,
        encoded_data: &[u8],
        gossipsub_tag: GossipTopicTag,
    ) -> Result<Self::ResponseMessage, io::Error> {
        let decoded_response = match gossipsub_tag {
            GossipTopicTag::NewTx => {
                GossipsubMessage::NewTx(self.deserialize(encoded_data)?)
            }
        };

        Ok(decoded_response)
    }
}

impl RequestResponseConverter for PostcardCodec {
    type NetworkResponse = NetworkResponse;
    type OutboundResponse = OutboundResponse;
    type ResponseMessage = ResponseMessage;

    fn convert_to_response(
        &self,
        inter_msg: &Self::NetworkResponse,
    ) -> Result<Self::ResponseMessage, io::Error> {
        match inter_msg {
            NetworkResponse::Block(block_bytes) => {
                let response = if let Some(block_bytes) = block_bytes {
                    Some(self.deserialize(block_bytes)?)
                } else {
                    None
                };

                Ok(ResponseMessage::SealedBlock(Box::new(response)))
            }
            NetworkResponse::Transactions(tx_bytes) => {
                let response = if let Some(tx_bytes) = tx_bytes {
                    Some(self.deserialize(tx_bytes)?)
                } else {
                    None
                };

                Ok(ResponseMessage::Transactions(response))
            }
            NetworkResponse::Headers(headers_bytes) => {
                let response = headers_bytes
                    .as_ref()
                    .map(|bytes| self.deserialize(bytes))
                    .transpose()?;
                Ok(ResponseMessage::SealedHeaders(response))
            }
        }
    }

    fn convert_to_network_response(
        &self,
        res_msg: &Self::OutboundResponse,
    ) -> Result<Self::NetworkResponse, io::Error> {
        match res_msg {
            OutboundResponse::Block(sealed_block) => {
                let response = if let Some(sealed_block) = sealed_block {
                    Some(self.serialize(sealed_block.as_ref())?)
                } else {
                    None
                };

                Ok(NetworkResponse::Block(response))
            }
            OutboundResponse::Transactions(transactions) => {
                let response = if let Some(transactions) = transactions {
                    Some(self.serialize(transactions.as_ref())?)
                } else {
                    None
                };

                Ok(NetworkResponse::Transactions(response))
            }
            OutboundResponse::SealedHeaders(maybe_headers) => {
                let response = maybe_headers
                    .as_ref()
                    .map(|headers| self.serialize(&headers))
                    .transpose()?;
                Ok(NetworkResponse::Headers(response))
            }
        }
    }
}

impl NetworkCodec for PostcardCodec {
    fn get_req_res_protocol(&self) -> <Self as RequestResponseCodec>::Protocol {
        MessageExchangePostcardProtocol {}
    }
}

#[derive(Default, Debug, Clone)]
pub struct MessageExchangePostcardProtocol;

impl AsRef<str> for MessageExchangePostcardProtocol {
    fn as_ref(&self) -> &str {
        REQUEST_RESPONSE_PROTOCOL_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_response::messages::MAX_REQUEST_SIZE;

    #[test]
    fn test_request_size_fits() {
        let arbitrary_range = 2..6;
        let m = RequestMessage::Transactions(arbitrary_range);
        assert!(postcard::to_stdvec(&m).unwrap().len() <= MAX_REQUEST_SIZE);
    }
}
