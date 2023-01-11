use std::{
    ops::RangeInclusive,
    sync::Arc,
};

use fuel_core_services::{
    SharedMutex,
    Shutdown,
    SourcePeer,
};
use fuel_core_types::blockchain::{
    block::Block,
    consensus::Sealed,
    SealedBlock,
    SealedBlockHeader,
};
use futures::{
    stream,
    stream::StreamExt,
    Stream,
};
use std::future::Future;
use tokio::sync::Notify;

use crate::{
    ports::{
        Executor,
        PeerToPeer,
    },
    State,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub max_get_header_requests: usize,
    pub max_get_txns_requests: usize,
}

pub(super) async fn import(
    state: SharedMutex<State>,
    notify: Arc<Notify>,
    params: Params,
    p2p: Arc<impl PeerToPeer + 'static>,
    executor: Arc<impl Executor + 'static>,
    shutdown: Shutdown,
) {
    loop {
        if let Some(range) = state.apply(|s| {
            match (s.in_flight_height.as_mut(), s.best_seen_height.as_mut()) {
                (Some(in_flight), Some(best)) if !in_flight.is_empty() => {
                    in_flight.end = *best;
                    Some(in_flight.start..=*best)
                }
                (Some(in_flight), None) if !in_flight.is_empty() => {
                    Some(in_flight.start..=in_flight.end)
                }
                (Some(in_flight), Some(best)) if *best > in_flight.end => {
                    in_flight.start = in_flight.end + 1u32.into();
                    in_flight.end = *best;
                    Some(in_flight.start..=in_flight.end)
                }
                (None, Some(best)) => {
                    s.in_flight_height = Some(0u32.into()..*best);
                    Some(0u32.into()..=*best)
                }
                _ => None,
            }
        }) {
            let range = (**range.start())..=(**range.end());
            get_header_range_buffered(range, params, p2p.clone())
                .map(|header| {
                    let SourcePeer {
                        peer_id,
                        data:
                            Sealed {
                                entity: header,
                                consensus,
                            },
                    } = header;
                    let id = header.id();
                    let block_id = SourcePeer { peer_id, data: id };
                    let p2p = p2p.clone();
                    async move {
                        p2p.get_transactions(block_id).await.unwrap().and_then(
                            |transactions| {
                                Some(SealedBlock {
                                    entity: Block::try_from_executed(
                                        header,
                                        transactions,
                                    )?,
                                    consensus,
                                })
                            },
                        )
                    }
                })
                .buffered(params.max_get_txns_requests)
                .scan((), |_, block| futures::future::ready(block))
                .take_until({
                    let s = shutdown.clone();
                    async move { s.wait().await }
                })
                .for_each(|block| {
                    let state = state.clone();
                    let height = *block.entity.header().height();
                    let executor = executor.clone();
                    async move {
                        match executor.execute_and_commit(block).await {
                            Ok(_) => {
                                state.apply(|s| {
                                    s.in_flight_height.as_mut().unwrap().start = height;
                                });
                            }
                            Err(_) => todo!(),
                        }
                    }
                })
                .await;
        }
        let n = notify.notified();
        let s = shutdown.wait();
        futures::pin_mut!(n);
        futures::pin_mut!(s);
        let s = futures::future::select(n, s).await;
        if let futures::future::Either::Right(_) = s {
            return
        }
    }
}

fn get_header_range_buffered(
    range: RangeInclusive<u32>,
    params: Params,
    p2p: Arc<impl PeerToPeer + 'static>,
) -> impl Stream<Item = SourcePeer<SealedBlockHeader>> {
    get_header_range(range, p2p)
        .buffered(params.max_get_header_requests)
        .scan((), |_, h| futures::future::ready(h))
}

fn get_header_range(
    range: RangeInclusive<u32>,
    p2p: Arc<impl PeerToPeer + 'static>,
) -> impl Stream<Item = impl Future<Output = Option<SourcePeer<SealedBlockHeader>>>> {
    stream::iter(range).map(move |height| {
        let p2p = p2p.clone();
        async move { p2p.get_sealed_block_header(height.into()).await.unwrap() }
    })
}
