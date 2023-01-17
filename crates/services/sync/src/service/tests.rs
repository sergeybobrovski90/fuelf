use fuel_core_services::Service;
use fuel_core_types::{
    blockchain::block::Block,
    services::executor::ExecutionResult,
};
use futures::{
    stream,
    StreamExt,
};

use crate::{
    import::empty_header,
    ports::{
        MockConsensusPort,
        MockExecutorPort,
        MockPeerToPeerPort,
    },
};

use super::*;

#[tokio::test]
async fn test_new_service() {
    let mut p2p = MockPeerToPeerPort::default();
    p2p.expect_height_stream().returning(|| {
        stream::iter(
            std::iter::successors(Some(6u32), |n| Some(n + 1)).map(BlockHeight::from),
        )
        .then(|h| async move {
            if *h == 17 {
                futures::future::pending::<()>().await;
            }
            h
        })
        .boxed()
    });
    p2p.expect_get_sealed_block_header()
        .returning(|h| Ok(Some(empty_header(h))));
    p2p.expect_get_transactions()
        .returning(|_| Ok(Some(vec![])));
    let mut executor = MockExecutorPort::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    executor.expect_execute_and_commit().returning(move |h| {
        tx.try_send(**h.entity.header().height()).unwrap();
        Ok(ExecutionResult {
            block: Block::default(),
            skipped_transactions: vec![],
            tx_status: vec![],
        })
    });
    let mut consensus = MockConsensusPort::default();
    consensus
        .expect_check_sealed_header()
        .returning(|_| Ok(true));
    let params = Params {
        max_get_header_requests: 10,
        max_get_txns_requests: 10,
    };
    let s = new_service(4u32.into(), p2p, executor, consensus, params).unwrap();

    assert_eq!(
        s.start_and_await().await.unwrap(),
        fuel_core_services::State::Started
    );
    while let Some(h) = rx.recv().await {
        if h == 16 {
            break
        }
    }

    assert_eq!(
        s.stop_and_await().await.unwrap(),
        fuel_core_services::State::Stopped
    );
}
