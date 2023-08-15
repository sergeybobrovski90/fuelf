use std::{
    ops::Range,
    time::Duration,
};

use fuel_core_services::stream::BoxStream;
use fuel_core_types::{
    blockchain::primitives::{
        BlockId,
        DaBlockHeight,
    },
    fuel_tx::Transaction,
};

use crate::ports::{
    BlockImporterPort,
    MockBlockImporterPort,
    MockConsensusPort,
    MockPeerToPeerPort,
};

use super::{
    tests::empty_header,
    *,
};
use fuel_core_types::fuel_types::BlockHeight;
use test_case::test_case;

#[derive(Default)]
struct Input {
    headers: Duration,
    consensus: Duration,
    transactions: Duration,
    executes: Duration,
}

#[test_case(
    Input::default(), State::new(None, None),
    Config{
        max_get_txns_requests: 1,
        header_batch_size: 1,
        max_header_batch_requests: 1,
    }
    => Count::default() ; "Empty sanity test"
)]
#[test_case(
    Input {
        headers: Duration::from_millis(10),
        ..Default::default()
    },
    State::new(None, 0),
    Config{
        max_get_txns_requests: 1,
        header_batch_size: 1,
        max_header_batch_requests: 1,
    }
    => is less_or_equal_than Count{ headers: 1, consensus: 1, transactions: 1, executes: 1, blocks: 1 }
    ; "Single with slow headers"
)]
#[test_case(
    Input {
        headers: Duration::from_millis(10),
        ..Default::default()
    },
    State::new(None, 100),
    Config{
        max_get_txns_requests: 10,
        header_batch_size: 10,
        max_header_batch_requests: 1,
    }
    => is less_or_equal_than Count{ headers: 10, consensus: 10, transactions: 10, executes: 1, blocks: 21 }
    ; "100 headers with max 10 with slow headers"
)]
#[test_case(
    Input {
        transactions: Duration::from_millis(10),
        ..Default::default()
    },
    State::new(None, 100),
    Config{
        max_get_txns_requests: 10,
        header_batch_size: 10,
        max_header_batch_requests: 1,
    }
    => is less_or_equal_than Count{ headers: 10, consensus: 10, transactions: 10, executes: 1, blocks: 21 }
    ; "100 headers with max 10 with slow transactions"
)]
#[test_case(
    Input {
        executes: Duration::from_millis(10),
        ..Default::default()
    },
    State::new(None, 50),
    Config{
        max_get_txns_requests: 10,
        header_batch_size: 10,
        max_header_batch_requests: 1,
    }
    => is less_or_equal_than Count{ headers: 10, consensus: 10, transactions: 10, executes: 1, blocks: 21 }
    ; "50 headers with max 10 with slow executes"
)]
#[test_case(
Input {
executes: Duration::from_millis(10),
..Default::default()
},
State::new(None, 50),
Config{
max_get_txns_requests: 10,
header_batch_size: 10,
max_header_batch_requests: 10,
}
=> is less_or_equal_than Count{ headers: 10, consensus: 10, transactions: 10, executes: 1, blocks: 21 }
; "50 headers with max 10 size and max 10 requests"
)]
#[tokio::test(flavor = "multi_thread")]
async fn test_back_pressure(input: Input, state: State, params: Config) -> Count {
    let counts = SharedCounts::new(Default::default());
    let state = SharedMutex::new(state);

    let p2p = Arc::new(PressurePeerToPeer::new(
        counts.clone(),
        [input.headers, input.transactions],
    ));
    let executor = Arc::new(PressureBlockImporter::new(counts.clone(), input.executes));
    let consensus = Arc::new(PressureConsensus::new(counts.clone(), input.consensus));
    let notify = Arc::new(Notify::new());

    let import = Import {
        state,
        notify,
        params,
        p2p,
        executor,
        consensus,
    };

    import.notify.notify_one();
    let (_tx, shutdown) = tokio::sync::watch::channel(fuel_core_services::State::Started);
    let mut watcher = shutdown.into();
    import.import(&mut watcher).await.unwrap();
    counts.apply(|c| c.max.clone())
}

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct Count {
    headers: usize,
    transactions: usize,
    consensus: usize,
    executes: usize,
    blocks: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    now: Count,
    max: Count,
}

type SharedCounts = SharedMutex<Counts>;

struct PressurePeerToPeer {
    p2p: MockPeerToPeerPort,
    durations: [Duration; 2],
    counts: SharedCounts,
}

struct PressureBlockImporter(MockBlockImporterPort, Duration, SharedCounts);

struct PressureConsensus(MockConsensusPort, Duration, SharedCounts);

#[async_trait::async_trait]
impl PeerToPeerPort for PressurePeerToPeer {
    fn height_stream(&self) -> BoxStream<BlockHeight> {
        self.p2p.height_stream()
    }

    async fn get_sealed_block_headers(
        &self,
        block_height_range: Range<u32>,
    ) -> anyhow::Result<Vec<SourcePeer<SealedBlockHeader>>> {
        self.counts.apply(|c| c.inc_headers());
        tokio::time::sleep(self.durations[0]).await;
        self.counts.apply(|c| c.dec_headers());
        for _ in block_height_range.clone() {
            self.counts.apply(|c| c.inc_blocks());
        }
        self.p2p.get_sealed_block_headers(block_height_range).await
    }

    async fn get_transactions(
        &self,
        block_id: SourcePeer<BlockId>,
    ) -> anyhow::Result<Option<Vec<Transaction>>> {
        self.counts.apply(|c| c.inc_transactions());
        tokio::time::sleep(self.durations[1]).await;
        self.counts.apply(|c| c.dec_transactions());
        self.p2p.get_transactions(block_id).await
    }
}

#[async_trait::async_trait]
impl BlockImporterPort for PressureBlockImporter {
    fn committed_height_stream(&self) -> BoxStream<BlockHeight> {
        self.0.committed_height_stream()
    }

    async fn execute_and_commit(&self, block: SealedBlock) -> anyhow::Result<()> {
        self.2.apply(|c| c.inc_executes());
        tokio::time::sleep(self.1).await;
        self.2.apply(|c| {
            c.dec_executes();
            c.dec_blocks();
        });
        self.0.execute_and_commit(block).await
    }
}

#[async_trait::async_trait]
impl ConsensusPort for PressureConsensus {
    fn check_sealed_header(&self, header: &SealedBlockHeader) -> anyhow::Result<bool> {
        self.0.check_sealed_header(header)
    }

    async fn await_da_height(&self, da_height: &DaBlockHeight) -> anyhow::Result<()> {
        self.2.apply(|c| c.inc_consensus());
        tokio::time::sleep(self.1).await;
        self.2.apply(|c| c.dec_consensus());
        self.0.await_da_height(da_height).await
    }
}

impl PressurePeerToPeer {
    fn new(counts: SharedCounts, delays: [Duration; 2]) -> Self {
        let mut mock = MockPeerToPeerPort::default();
        mock.expect_get_sealed_block_headers().returning(|range| {
            Ok(range
                .clone()
                .map(BlockHeight::from)
                .map(empty_header)
                .collect())
        });
        mock.expect_get_transactions()
            .returning(|_| Ok(Some(vec![])));
        Self {
            p2p: mock,
            durations: delays,
            counts,
        }
    }
}

impl PressureBlockImporter {
    fn new(counts: SharedCounts, delays: Duration) -> Self {
        let mut mock = MockBlockImporterPort::default();
        mock.expect_execute_and_commit().returning(move |_| Ok(()));
        Self(mock, delays, counts)
    }
}

impl PressureConsensus {
    fn new(counts: SharedCounts, delays: Duration) -> Self {
        let mut mock = MockConsensusPort::default();
        mock.expect_await_da_height().returning(|_| Ok(()));
        mock.expect_check_sealed_header().returning(|_| Ok(true));
        Self(mock, delays, counts)
    }
}

impl Counts {
    fn inc_headers(&mut self) {
        self.now.headers += 1;
        self.max.headers = self.max.headers.max(self.now.headers);
    }
    fn dec_headers(&mut self) {
        self.now.headers -= 1;
    }
    fn inc_transactions(&mut self) {
        self.now.transactions += 1;
        self.max.transactions = self.max.transactions.max(self.now.transactions);
    }
    fn dec_transactions(&mut self) {
        self.now.transactions -= 1;
    }
    fn inc_consensus(&mut self) {
        self.now.consensus += 1;
        self.max.consensus = self.max.consensus.max(self.now.consensus);
    }
    fn dec_consensus(&mut self) {
        self.now.consensus -= 1;
    }
    fn inc_executes(&mut self) {
        self.now.executes += 1;
        self.max.executes = self.max.executes.max(self.now.executes);
    }
    fn dec_executes(&mut self) {
        self.now.executes -= 1;
    }
    fn inc_blocks(&mut self) {
        self.now.blocks += 1;
        self.max.blocks = self.max.blocks.max(self.now.blocks);
    }
    fn dec_blocks(&mut self) {
        self.now.blocks -= 1;
    }
}
